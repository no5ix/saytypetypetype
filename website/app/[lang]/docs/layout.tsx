import { DocsLayout } from "fumadocs-ui/layouts/docs"
import { source } from "@/lib/source"
import { notFound } from "next/navigation"
import { isLocale } from "@/lib/i18n"
export default async function Layout({
  children,
  params,
}: {
  children: React.ReactNode
  params: Promise<{ lang: string }>
}) {
  const { lang } = await params
  if (!isLocale(lang)) notFound()
  return (
    <DocsLayout
      tree={source.getPageTree(lang)}
      nav={{
        title: lang === "en" ? "Documentation" : "配置文档",
        url: `/${lang}/docs`,
      }}
      themeSwitch={{ enabled: false }}
      i18n={false}
    >
      {children}
    </DocsLayout>
  )
}
