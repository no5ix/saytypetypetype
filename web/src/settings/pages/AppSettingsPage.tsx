import { useState } from "react";
import { useT } from "@/i18n";
import { selectSoundFile } from "@/settings/bridge";
import { Button } from "@/settings/components/Button";
import { SegmentedControl } from "@/settings/components/SegmentedControl";
import { Toggle } from "@/settings/components/Toggle";
import {
  PageHeader,
  PageLayout,
  Section,
  SectionContent,
  SectionItem,
  SectionItemList,
} from "@/settings/layout/PageLayout";
import { soundFileName } from "@/settings/lib/sound";
import { useSettings } from "@/settings/SettingsProvider";

export function AppSettingsPage() {
  const { settings, scheduleSave } = useSettings();
  const t = useT();
  const cfg = settings?.parsedConfig || ({} as Record<string, unknown>);
  const app = (cfg.app || {}) as Record<string, unknown>;
  const sound = (app.sound || {}) as Record<string, boolean | string>;
  const runtime = settings?.runtime || ({} as Record<string, unknown>);
  const platform = (runtime.platform as string) || "";
  const configPath = settings?.configPath || "-";
  const isMac = platform === "macos";

  const [startSound, setStartSound] = useState<string>((sound.start_sound as string) || "");
  const [endSound, setEndSound] = useState<string>((sound.end_sound as string) || "");

  const setAppBool = (key: string, val: boolean) => scheduleSave({ app: { [key]: val } });
  const setSound = (upd: Record<string, unknown>) => scheduleSave({ app: { sound: upd } });

  return (
    <PageLayout>
      <PageHeader title={t("应用设置")} description={t("启动行为与配置文件")} />

      <Section>
        <SectionContent>
          <SectionItemList>
            <SectionItem
              title={t("界面语言")}
              description={t("界面语言说明")}
              action={
                <SegmentedControl
                  options={[
                    { value: "system", label: t("跟随系统") },
                    { value: "zh-CN", label: t("简体中文") },
                    { value: "en", label: "English" },
                  ]}
                  value={(app.language as string) || "system"}
                  onChange={(v) => scheduleSave({ app: { language: v } })}
                />
              }
              className="shrink-0"
            />

            <SectionItem
              title={t("开机自动启动")}
              action={
                <Toggle checked={!!app.autoLaunch} onChange={(v) => setAppBool("autoLaunch", v)} />
              }
            />

            {isMac && (
              <SectionItem
                title={t("悬浮窗外观")}
                tooltip={t(
                  "通透：背景透出更多、液态感更强\n磨砂：磨砂感更明显\n跟随主题：传统毛玻璃、明暗跟随外观主题",
                )}
                action={
                  <SegmentedControl
                    options={[
                      { value: "liquid", label: t("通透") },
                      { value: "liquid-standard", label: t("磨砂") },
                      { value: "vibrancy", label: t("跟随主题") },
                    ]}
                    value={(app.overlay_style as string) || "liquid"}
                    onChange={(v) => scheduleSave({ app: { overlay_style: v } })}
                  />
                }
                className="shrink-0"
              />
            )}

            <SectionItem last title={t("配置文件路径")} description={configPath} />
          </SectionItemList>
        </SectionContent>
      </Section>

      <Section>
        <SectionContent>
          <SectionItemList>
            <SectionItem
              title={t("提示音")}
              action={
                <Toggle
                  checked={sound.enabled !== false}
                  onChange={(v) => setSound({ enabled: v })}
                />
              }
            />

            {sound.enabled !== false && (
              <>
                <SectionItem
                  title={t("就绪提示音")}
                  action={
                    <div className="flex items-center gap-2">
                      <span className="text-xs text-text-muted max-w-40 truncate">
                        {t(soundFileName(startSound))}
                      </span>
                      <Button
                        onClick={async () => {
                          const path = await selectSoundFile();
                          if (path) {
                            setStartSound(path);
                            setSound({ start_sound: path });
                          }
                        }}
                      >
                        {t("选择文件")}
                      </Button>
                      {startSound && (
                        <Button
                          variant="ghost"
                          onClick={() => {
                            setStartSound("");
                            setSound({ start_sound: "" });
                          }}
                        >
                          {t("重置")}
                        </Button>
                      )}
                    </div>
                  }
                />

                <SectionItem
                  title={t("完成提示音")}
                  last
                  action={
                    <div className="flex items-center gap-2">
                      <span className="text-xs text-text-muted max-w-40 truncate">
                        {t(soundFileName(endSound))}
                      </span>
                      <Button
                        onClick={async () => {
                          const path = await selectSoundFile();
                          if (path) {
                            setEndSound(path);
                            setSound({ end_sound: path });
                          }
                        }}
                      >
                        {t("选择文件")}
                      </Button>
                      {endSound && (
                        <Button
                          variant="ghost"
                          onClick={() => {
                            setEndSound("");
                            setSound({ end_sound: "" });
                          }}
                        >
                          {t("重置")}
                        </Button>
                      )}
                    </div>
                  }
                />
              </>
            )}
          </SectionItemList>
        </SectionContent>
      </Section>

      <Section>
        <SectionContent>
          <SectionItemList>
            <SectionItem
              title={t("移除末尾句号")}
              tooltip={t("粘贴前自动移除识别结果末尾的句号")}
              action={
                <Toggle
                  checked={app.remove_trailing_period !== false}
                  onChange={(v) => setAppBool("remove_trailing_period", v)}
                />
              }
            />
            {isMac && (
              <SectionItem
                title={t("多版本转写选择")}
                tooltip={t(
                  "识别完成后弹出选择面板：原文 + 每个处理模板的结果\n使用对应快捷键或点击卡片粘贴，Esc 取消\n关闭时保持原有的自动粘贴流程（需 macOS）",
                )}
                action={
                  <Toggle
                    checked={app.multi_version_picker === true}
                    onChange={(v) => setAppBool("multi_version_picker", v)}
                  />
                }
              />
            )}
            <SectionItem
              title={t("保留剪贴板")}
              tooltip={t("粘贴后保留识别文字在剪贴板；关闭则恢复原剪贴板内容")}
              action={
                <Toggle
                  checked={app.keep_clipboard !== false}
                  onChange={(v) => setAppBool("keep_clipboard", v)}
                />
              }
            />
            <SectionItem
              title={t("保留录音")}
              tooltip={t("开启后保留最近 1 个月录音；关闭时仅保留失败录音用于重试")}
              last
              action={
                <Toggle
                  checked={app.keep_recordings === true}
                  onChange={(v) => setAppBool("keep_recordings", v)}
                />
              }
            />
          </SectionItemList>
        </SectionContent>
      </Section>
    </PageLayout>
  );
}
