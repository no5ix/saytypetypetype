import { PlatformIcon } from "./platform-icon"
import { buttonVariants } from "@/components/ui/button"
import {
  latest,
  asset,
  platformNames,
  type Platform,
  type Release,
} from "@/lib/releases"
import { cn } from "@/lib/utils"
export function DownloadButton({
  platform,
  secondary = false,
  release = latest,
}: {
  platform: Platform
  secondary?: boolean
  release?: Release
}) {
  const file = asset(release, platform)
  return (
    <a
      data-download={platform}
      href={file.url}
      className={cn(
        buttonVariants({
          variant: secondary ? "secondary" : "default",
          size: "lg",
        }),
        "min-h-12 gap-2 px-5"
      )}
    >
      <PlatformIcon platform={platform} />
      {platformNames[platform]}
    </a>
  )
}
