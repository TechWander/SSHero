//! Remote filesystem operations over a shared SFTP session: browsing,
//! mutation, streaming download (single file / tar.gz folder) and upload.

use std::{path::PathBuf, sync::Arc};

use anyhow::{anyhow, Result};
use axum::body::Body;
use bytes::Bytes;
use flate2::write::GzEncoder;
use futures_util::{Stream, StreamExt};
use russh_sftp::{
    client::SftpSession,
    protocol::OpenFlags,
};
use serde::Serialize;
use tar::{Builder, EntryType, Header};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio_util::io::{ReaderStream, SyncIoBridge};

/// One directory entry as exposed to the frontend.
#[derive(Debug, Serialize)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    /// unix mtime seconds, 0 when unknown
    pub mtime: u64,
    /// raw permission + type bits from the server
    pub mode: u32,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

/// Join remote POSIX path segments, collapsing duplicate slashes.
pub fn remote_join(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        let mut s = dir.trim_end_matches('/').to_string();
        s.push('/');
        s.push_str(name.trim_start_matches('/'));
        s
    }
}

fn entry_from(parent: &str, name: String, m: &russh_sftp::client::fs::Metadata) -> RemoteEntry {
    RemoteEntry {
        name: name.clone(),
        path: remote_join(parent, &name),
        is_dir: m.is_dir(),
        is_symlink: m.is_symlink(),
        size: m.len(),
        mtime: m.mtime.unwrap_or(0) as u64,
        mode: m.permissions.unwrap_or(0),
        uid: m.uid,
        gid: m.gid,
    }
}

/// List a remote directory. `path` is canonicalized by the server when
/// relative ("" or "." mean the user's home directory).
pub async fn list_dir(sftp: &SftpSession, path: &str) -> Result<(String, Vec<RemoteEntry>)> {
    let canon = sftp
        .canonicalize(if path.is_empty() { "." } else { path })
        .await
        .map_err(|e| anyhow!("canonicalize {path:?} failed: {e}"))?;

    let mut rd = sftp
        .read_dir(&canon)
        .await
        .map_err(|e| anyhow!("read_dir {canon:?} failed: {e}"))?;

    let mut entries = Vec::new();
    while let Some(de) = rd.next() {
        let name = de.file_name();
        let meta = de.metadata();
        entries.push(entry_from(&canon, name, &meta));
    }
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok((canon, entries))
}

pub async fn stat(sftp: &SftpSession, path: &str) -> Result<RemoteEntry> {
    let md = sftp
        .symlink_metadata(path)
        .await
        .map_err(|e| anyhow!("stat {path:?} failed: {e}"))?;
    let name = pathbuf_name(path).unwrap_or_else(|| path.to_string());
    Ok(entry_from(&parent_of(path), name, &md))
}

fn pathbuf_name(path: &str) -> Option<String> {
    PathBuf::from(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
}

/// Pure helper: parent directory of a remote POSIX path ("/a/b" -> "/a").
pub fn parent_of(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => trimmed[..i].to_string(),
        None => ".".to_string(),
    }
}

pub async fn mkdir_one(sftp: &SftpSession, path: &str) -> Result<()> {
    sftp.create_dir(path)
        .await
        .map_err(|e| anyhow!("mkdir {path:?} failed: {e}"))
}

/// Create `path` and all missing parents (like `mkdir -p`).
pub async fn mkdir_p(sftp: &SftpSession, path: &str) -> Result<()> {
    let mut accum = if path.starts_with('/') {
        String::new()
    } else {
        ".".to_string()
    };
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            if accum == "." {
                continue;
            }
            accum = parent_of(&accum);
            continue;
        }
        accum = remote_join(&accum, seg);
        match sftp.metadata(&accum).await {
            Ok(md) if md.is_dir() => continue,
            Ok(_) => {
                return Err(anyhow!("{accum:?} exists and is not a directory"));
            }
            Err(_) => {}
        }
        if let Err(e) = mkdir_one(sftp, &accum).await {
            // tolerate races / servers that reject exist
            if let Ok(md) = sftp.metadata(&accum).await {
                if md.is_dir() {
                    continue;
                }
            }
            return Err(e);
        }
    }
    Ok(())
}

pub async fn remove(sftp: &SftpSession, path: &str, recursive: bool) -> Result<()> {
    let md = sftp
        .symlink_metadata(path)
        .await
        .map_err(|e| anyhow!("stat {path:?} failed: {e}"))?;
    if md.is_dir() && !md.is_symlink() {
        if recursive {
            let mut stack = vec![path.to_string()];
            // collect everything bottom-up
            let mut all_dirs = Vec::new();
            let mut all_files = Vec::new();
            while let Some(dir) = stack.pop() {
                all_dirs.push(dir.clone());
                let mut rd = sftp
                    .read_dir(&dir)
                    .await
                    .map_err(|e| anyhow!("read_dir {dir:?} failed: {e}"))?;
                while let Some(de) = rd.next() {
                    let child = de.path();
                    let cm = de.metadata();
                    if cm.is_dir() && !cm.is_symlink() {
                        stack.push(child);
                    } else {
                        all_files.push(child);
                    }
                }
            }
            for f in all_files {
                sftp.remove_file(&f)
                    .await
                    .map_err(|e| anyhow!("remove {f:?} failed: {e}"))?;
            }
            all_dirs.reverse(); // children before parents
            for d in all_dirs {
                sftp.remove_dir(&d)
                    .await
                    .map_err(|e| anyhow!("rmdir {d:?} failed: {e}"))?;
            }
            return Ok(());
        }
        sftp.remove_dir(path)
            .await
            .map_err(|e| anyhow!("rmdir {path:?} failed: {e} (is it non-empty? use recursive)"))
    } else {
        sftp.remove_file(path)
            .await
            .map_err(|e| anyhow!("remove {path:?} failed: {e}"))
    }
}

pub async fn rename(sftp: &SftpSession, from: &str, to: &str) -> Result<()> {
    sftp.rename(from, to)
        .await
        .map_err(|e| anyhow!("rename {from:?} -> {to:?} failed: {e}"))
}

/// Stream a single remote file as an HTTP body, plus its size when known.
pub async fn download_file(sftp: Arc<SftpSession>, path: String) -> Result<(Body, Option<u64>)> {
    let md = sftp
        .metadata(&path)
        .await
        .map_err(|e| anyhow!("stat {path:?} failed: {e}"))?;
    let size = Some(md.len());
    let file = sftp
        .open(&path)
        .await
        .map_err(|e| anyhow!("open {path:?} failed: {e}"))?;

    let reader = BufReader::with_capacity(256 * 1024, file);
    let stream = ReaderStream::new(reader);
    let body = Body::from_stream(stream);
    Ok((body, size))
}

/// Recursively fetch a remote directory and stream it back as `.tar.gz`.
///
/// Note (honest limitation of v1): each file is buffered fully in memory
/// before being written into the archive, so peak RSS ≈ largest single file.
pub async fn download_dir_tar_gz(
    sftp: Arc<SftpSession>,
    root: String,
) -> Result<(Body, Option<u64>)> {
    let root_canon = sftp
        .canonicalize(&root)
        .await
        .map_err(|e| anyhow!("canonicalize {root:?} failed: {e}"))?;
    let archive_name = pathbuf_name(&root_canon)
        .unwrap_or_else(|| "download".to_string())
        .replace('/', "_");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<(String, Vec<u8>)>(2);
    let (client_half, server_half) = tokio::io::duplex(128 * 1024);
    let (err_tx, mut err_rx) = tokio::sync::mpsc::channel::<String>(1);

    // writer thread: turns pushed files into tar.gz on the pipe
    let writer_task = tokio::task::spawn_blocking(move || -> Result<()> {
        let sync = SyncIoBridge::new(server_half);
        let mut gz = GzEncoder::new(sync, flate2::Compression::default());
        {
            let mut tb = Builder::new(&mut gz);
            // directory header for the archive root (no local fs lookup needed)
            let mut dhd = Header::new_gnu();
            dhd.set_mode(0o755);
            dhd.set_size(0);
            dhd.set_mtime(0);
            dhd.set_entry_type(EntryType::Directory);
            tb.append_data(&mut dhd, format!("{archive_name}/"), &[][..])?;
            while let Some((name, data)) = rx.blocking_recv() {
                let mut hd = Header::new_gnu();
                hd.set_mode(0o644);
                hd.set_size(data.len() as u64);
                hd.set_entry_type(EntryType::Regular);
                hd.set_mtime(0);
                tb.append_data(&mut hd, format!("{archive_name}/{name}"), &data[..])?;
            }
            tb.finish()?;
        }
        gz.finish()?;
        Ok(())
    });

    // walker: recursive listing streamed into the channel
    let walker = {
        let sftp = sftp.clone();
        tokio::spawn(async move {
            let res: Result<()> = async {
                let mut stack = vec![(root_canon.clone(), String::new())];
                while let Some((dir, prefix)) = stack.pop() {
                    let mut rd = sftp
                        .read_dir(&dir)
                        .await
                        .map_err(|e| anyhow!("read_dir {dir:?}: {e}"))?;
                    while let Some(de) = rd.next() {
                        let child = de.path();
                        let meta = de.metadata();
                        let rel = if prefix.is_empty() {
                            de.file_name()
                        } else {
                            format!("{prefix}/{}", de.file_name())
                        };
                        if meta.is_dir() && !meta.is_symlink() {
                            stack.push((child, rel));
                        } else {
                            let mut buf = Vec::new();
                            let mut f = sftp
                                .open(&child)
                                .await
                                .map_err(|e| anyhow!("open {child:?}: {e}"))?;
                            f.read_to_end(&mut buf)
                                .await
                                .map_err(|e| anyhow!("read {child:?}: {e}"))?;
                            if tx.send((rel, buf)).await.is_err() {
                                return Err(anyhow!("archive writer stopped"));
                            }
                        }
                    }
                }
                drop(tx);
                Ok(())
            }
            .await;
            if let Err(e) = res {
                let _ = err_tx.send(format!("{e:#}")).await;
            }
        })
    };

    let mut writer_task = writer_task;
    let mut err_closed = false;
    let stream = async_stream::stream! {
        let mut rs = ReaderStream::new(client_half);
        loop {
            tokio::select! {
                chunk = rs.next() => {
                    match chunk {
                        Some(Ok(b)) => yield Ok::<Bytes, anyhow::Error>(b),
                        Some(Err(e)) => { yield Err(anyhow!(e)); break; }
                        None => {
                            // reader done; ensure writer succeeded
                            if let Err(e) = (&mut writer_task).await {
                                yield Err(anyhow!("writer join: {e}"));
                            }
                            let _ = walker.await;
                            break;
                        }
                    }
                }
                e = err_rx.recv(), if !err_closed => {
                    match e {
                        Some(msg) => { yield Err(anyhow!(msg)); break; }
                        // all senders dropped without error: stop watching
                        None => err_closed = true,
                    }
                }
            }
        }
    };

    Ok((Body::from_stream(stream), None))
}

/// Write an incoming byte stream to `dest_path` on the remote, creating
/// parent directories.
pub async fn upload_stream<S, E>(sftp: &SftpSession, dest_dir: &str, rel_path: &str, mut body: S) -> Result<u64>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    let clean_rel = sanitize_rel_path(rel_path)?;
    let dest = remote_join(dest_dir, &clean_rel);
    mkdir_p(sftp, &parent_of(&dest)).await?;

    let mut file = sftp
        .open_with_flags(
            &dest,
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE,
        )
        .await
        .map_err(|e| anyhow!("open {dest:?} for write failed: {e}"))?;

    let mut written = 0u64;
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| anyhow!("reading request body: {e}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| anyhow!("write {dest:?} failed: {e}"))?;
        written += chunk.len() as u64;
    }
    file.flush().await.map_err(|e| anyhow!("flush {dest:?}: {e}"))?;
    file.close().await.map_err(|e| anyhow!("close {dest:?}: {e}"))?;
    Ok(written)
}

/// Reject absolute paths and `..` traversal in client-supplied relative paths.
pub fn sanitize_rel_path(rel: &str) -> Result<String> {
    let mut out = Vec::new();
    for seg in rel.split('/') {
        match seg {
            "" | "." => continue,
            ".." => return Err(anyhow!("relative path must not contain '..': {rel:?}")),
            s if s.contains('\0') => return Err(anyhow!("invalid NUL in path")),
            s => out.push(s.to_string()),
        }
    }
    if out.is_empty() {
        return Err(anyhow!("empty relative path"));
    }
    Ok(out.join("/"))
}
