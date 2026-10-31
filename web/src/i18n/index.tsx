import { createContext, useContext, useMemo } from "react";
import { EN } from "@/i18n/dictionary";

export type UiLanguage = "zh-CN" | "en";

export function normalizeLanguage(value: unknown): UiLanguage {
  return value === "en" ? "en" : "zh-CN";
}

export type Translate = (
  zh: string,
  vars?: Record<string, string | number | null | undefined>,
) => string;

function format(
  template: string,
  vars?: Record<string, string | number | null | undefined>,
): string {
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = vars[name];
    return value === undefined || value === null ? match : String(value);
  });
}

export function createTranslate(language: UiLanguage): Translate {
  return (zh, vars) => {
    if (language !== "en") return format(zh, vars);
    return format(EN[zh] ?? zh, vars);
  };
}

/** Translate without a React context — for windows that own their language
 *  state (the floating panel and picker receive it from the backend). */
export function translate(
  language: UiLanguage,
  zh: string,
  vars?: Record<string, string | number | null | undefined>,
): string {
  if (language !== "en") return format(zh, vars);
  return format(EN[zh] ?? zh, vars);
}

interface I18nContextValue {
  language: UiLanguage;
  t: Translate;
}

const I18nContext = createContext<I18nContextValue>({
  language: "zh-CN",
  t: createTranslate("zh-CN"),
});

export function I18nProvider({
  language,
  children,
}: {
  language: UiLanguage;
  children: React.ReactNode;
}) {
  const value = useMemo(() => ({ language, t: createTranslate(language) }), [language]);
  return <I18nContext value={value}>{children}</I18nContext>;
}

export function useI18n(): I18nContextValue {
  return useContext(I18nContext);
}

/** Convenience hook for components that only need the translator. */
export function useT(): Translate {
  return useI18n().t;
}
