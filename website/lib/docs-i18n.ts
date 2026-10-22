import { defineI18n } from "fumadocs-core/i18n"
import { uiTranslations } from "fumadocs-ui/i18n"
export const i18n = defineI18n({
  languages: ["en", "zh"],
  defaultLanguage: "en",
  fallbackLanguage: null,
})
export const translations = i18n
  .translations()
  .extend(uiTranslations())
  .add({
    en: { displayName: "English" },
    zh: {
      displayName: "中文",
      "Search(search dialog)": "搜索文档",
      "Search(search trigger)": "搜索文档",
      "Open Search(search trigger)(aria-label)": "搜索文档",
      "Close Search(search dialog)(aria-label)": "关闭搜索",
      "No results found(search dialog)": "未找到相关内容",
      "Search failed(search dialog)": "搜索失败",
      "On this page(table of contents)": "本页目录",
      "No Headings(table of contents)": "暂无标题",
      "Table of Contents(inline table of contents)": "本页目录",
      "Next Page(pagination)": "下一篇",
      "Previous Page(pagination)": "上一篇",
      "Open Sidebar(sidebar)(aria-label)": "展开目录",
      "Close Sidebar(sidebar)(aria-label)": "关闭目录",
      "Collapse Sidebar(sidebar)(aria-label)": "收起目录",
      "Toggle Menu(mobile menu)(aria-label)": "切换菜单",
      "Copy Anchor Link(heading anchor)(aria-label)": "复制标题链接",
    },
  })
