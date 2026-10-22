export const locales = ["en", "zh"] as const
export type Locale = (typeof locales)[number]
export function isLocale(value: string): value is Locale {
  return value === "en" || value === "zh"
}
export const localeCookie = "voicepaste-locale"
export function detectLocale(header: string | null, saved?: string): Locale {
  if (saved && isLocale(saved)) return saved
  const languages = (header ?? "")
    .split(",")
    .map((entry, index) => {
      const [tag, ...parameters] = entry.trim().toLowerCase().split(";")
      const quality = parameters.find((value) => value.trim().startsWith("q="))
      const q = quality ? Number(quality.trim().slice(2)) : 1
      return { tag, q: Number.isFinite(q) && q >= 0 && q <= 1 ? q : 0, index }
    })
    .filter((value) => value.q > 0)
    .sort((a, b) => b.q - a.q || a.index - b.index)
  for (const { tag } of languages) {
    if (tag === "zh" || tag.startsWith("zh-")) return "zh"
    if (tag === "en" || tag.startsWith("en-")) return "en"
  }
  return "en"
}
