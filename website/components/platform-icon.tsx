import type { Platform } from "@/lib/releases"
// Font Awesome Brands; source SVGs and attribution are kept in public/assets/brands.
const icons = {
  apple: {
    viewBox: "0 0 384 512",
    paths: [
      "M319.1 268.7c-.2-36.7 16.4-64.4 50-84.8-18.8-26.9-47.2-41.7-84.7-44.6-35.5-2.8-74.3 20.7-88.5 20.7-15 0-49.4-19.7-76.4-19.7-55.8 .9-115.1 44.5-115.1 133.2 0 26.2 4.8 53.3 14.4 81.2 12.8 36.7 59 126.7 107.2 125.2 25.2-.6 43-17.9 75.8-17.9 31.8 0 48.3 17.9 76.4 17.9 48.6-.7 90.4-82.5 102.6-119.3-65.2-30.7-61.7-90-61.7-91.9zM262.5 104.5c27.3-32.4 24.8-61.9 24-72.5-24.1 1.4-52 16.4-67.9 34.9-17.5 19.8-27.8 44.3-25.6 71.9 26.1 2 49.9-11.4 69.5-34.3z",
    ],
  },
  windows: {
    viewBox: "0 0 448 512",
    paths: [
      "M0 93.7l183.6-25.3 0 177.4-183.6 0 0-152.1zM0 418.3l183.6 25.3 0-175.2-183.6 0 0 149.9zm203.8 28l244.2 33.7 0-211.6-244.2 0 0 177.9zm0-380.6l0 180.1 244.2 0 0-213.8-244.2 33.7z",
    ],
  },
} as const
export function PlatformIcon({ platform }: { platform: Platform }) {
  const key = platform.startsWith("macos") ? "apple" : "windows"
  const icon = icons[key]
  return (
    <svg
      data-platform-icon={key}
      className="size-[18px] shrink-0"
      viewBox={icon.viewBox}
      fill="currentColor"
      aria-hidden="true"
    >
      {icon.paths.map((d) => (
        <path key={d} d={d} />
      ))}
    </svg>
  )
}
