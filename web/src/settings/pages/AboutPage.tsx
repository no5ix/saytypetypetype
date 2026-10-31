import { useState } from "react";
import { useT } from "@/i18n";
import { openUrl } from "@/settings/bridge";
import { Toggle } from "@/settings/components/Toggle";
import { UpdateButton } from "@/settings/components/UpdateButton";
import {
  PageHeader,
  PageLayout,
  Section,
  SectionContent,
  SectionItem,
  SectionItemList,
} from "@/settings/layout/PageLayout";
import { APP_NAME, PROJECT_URL } from "@/settings/lib/appInfo";
import { useSettings } from "@/settings/SettingsProvider";

export function AboutPage() {
  const t = useT();
  const { settings, scheduleSave } = useSettings();
  const cfg = settings?.parsedConfig || ({} as Record<string, unknown>);
  const app = (cfg.app || {}) as Record<string, unknown>;

  const [betaUpdates, setBetaUpdates] = useState(!!app.beta_updates);

  const version = (settings?.runtime?.version as string) || "2.0.0";

  return (
    <PageLayout>
      <PageHeader title={t("关于")} />
      {/* App info */}
      <div className="flex flex-col items-center space-y-2">
        <img className="w-16 h-16 rounded-lg" src="./icon.png" alt={APP_NAME} />

        <p className="text-sm font-semibold text-text">{APP_NAME}</p>
        <p className="text-xs text-text-muted max-w-sm text-center">
          {t("语音输入工具，将语音实时转为文字并输入到任意应用")}
        </p>
      </div>

      <Section>
        <SectionContent>
          <SectionItemList>
            <SectionItem
              title={t("当前版本")}
              action={<span className="text-sm text-text-muted">{version}</span>}
            />
            <SectionItem title={t("检查更新")} action={<UpdateButton />} />
            <SectionItem
              title={t("项目主页")}
              action={
                <button
                  type="button"
                  className="text-sm text-accent hover:underline cursor-pointer bg-transparent border-0 p-0 font-inherit"
                  onClick={() => openUrl(PROJECT_URL)}
                >
                  {PROJECT_URL.replace("https://", "")}
                </button>
              }
            />
            <SectionItem
              title={t("体验 Beta 版本")}
              last
              action={
                <Toggle
                  checked={betaUpdates}
                  onChange={(v) => {
                    setBetaUpdates(v);
                    scheduleSave({ app: { beta_updates: v } });
                  }}
                />
              }
            />
          </SectionItemList>
        </SectionContent>
      </Section>
    </PageLayout>
  );
}
