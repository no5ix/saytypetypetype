import { EN_COMMON } from "@/i18n/parts/common";
import { EN_HOME } from "@/i18n/parts/home";
import { EN_MODELS } from "@/i18n/parts/models";
import { EN_SETTINGS } from "@/i18n/parts/settings";
import { EN_SHORTCUTS } from "@/i18n/parts/shortcuts";

/**
 * English translations keyed by the Simplified Chinese source string.
 *
 * Using the Chinese text as the key keeps call sites readable (`t("取消")`)
 * and makes an untranslated string fall back to Chinese instead of rendering a
 * raw identifier. Placeholders use `{name}` and are substituted by `t`.
 *
 * Translations are split by area so the tables stay reviewable; later parts win
 * on duplicate keys, which only matters for shared nouns like "保存".
 */
const PARTS: Record<string, string>[] = [EN_COMMON, EN_HOME, EN_SETTINGS, EN_SHORTCUTS, EN_MODELS];

export const EN: Record<string, string> = Object.assign({}, ...PARTS);
