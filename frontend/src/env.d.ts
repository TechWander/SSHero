/// <reference types="vite/client" />

declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<{}, {}, any>
  export default component
}

// webkitdirectory is not in the standard lib DOM typings
interface HTMLInputElement {
  webkitdirectory?: boolean
}
