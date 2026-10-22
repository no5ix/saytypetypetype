import { notFound } from "next/navigation"
import { isLocale } from "@/lib/i18n"
import { pageMetadata } from "@/lib/seo"
import { HomeContent } from "@/components/home-content"
type Props = { params: Promise<{ lang: string }> }
export async function generateMetadata({ params }: Props) {
  const { lang } = await params
  if (!isLocale(lang)) notFound()
  return pageMetadata(
    lang,
    "",
    lang === "en" ? "Think freely. Speak naturally." : "慢慢想，随意说。",
    lang === "en"
      ? "Turn scattered thoughts into clear writing. A free, lightweight, open-source voice input app for macOS and Windows."
      : "把零散想法整理成清楚的文字。免费、轻量、开源的 macOS 与 Windows 语音输入工具。"
  )
}
export default async function Home({ params }: Props) {
  const { lang } = await params
  if (!isLocale(lang)) notFound()
  return <HomeContent locale={lang} />
}
