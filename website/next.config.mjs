import { createMDX } from "fumadocs-mdx/next"
import { fileURLToPath } from "node:url"
const withMDX = createMDX()
export default withMDX({
  reactStrictMode: true,
  turbopack: { root: fileURLToPath(new URL(".", import.meta.url)) },
})
