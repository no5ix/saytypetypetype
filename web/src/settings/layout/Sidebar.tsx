import {
  AudioLines,
  BookOpen,
  Bot,
  Home,
  Info,
  Keyboard,
  NotebookText,
  Settings2,
  ShieldCheck,
} from "lucide-react";
import { useT } from "@/i18n";
import { openHelpGuide } from "@/settings/bridge";
import { ThemeSelector } from "@/settings/components/ThemeSelector";
import { Toast, useToast } from "@/settings/components/Toast";
import { UpdateButton } from "@/settings/components/UpdateButton";

export type SectionId =
  | "home"
  | "app"
  | "permissions"
  | "hotkey"
  | "service"
  | "llm"
  | "hotwords"
  | "about";

const NAV_ITEMS: { id: SectionId; icon: typeof Home; label: string }[] = [
  { id: "home", icon: Home, label: "首页" },
  { id: "app", icon: Settings2, label: "应用设置" },
  { id: "permissions", icon: ShieldCheck, label: "系统权限" },
  { id: "hotkey", icon: Keyboard, label: "快捷键" },
  { id: "service", icon: AudioLines, label: "音频模型" },
  { id: "llm", icon: Bot, label: "大语言模型" },
  { id: "hotwords", icon: NotebookText, label: "热词库" },
];

interface SidebarProps {
  active: SectionId;
  onNavigate: (id: SectionId) => void;
}

export function Sidebar({ active, onNavigate }: SidebarProps) {
  const t = useT();
  const { toast, showToast } = useToast();

  const navBtn = (id: SectionId, Icon: typeof Home, label: string) => (
    <button
      key={id}
      type="button"
      onClick={() => onNavigate(id)}
      className={`w-full flex items-center gap-2.5 px-3 py-[7px] rounded-lg text-sm transition-colors ${
        active === id
          ? "bg-accent-soft text-accent font-medium"
          : "text-text-dim hover:bg-fill-hover hover:text-text"
      }`}
    >
      <Icon size={18} className="shrink-0" />
      <span className="min-w-0 whitespace-nowrap">{label}</span>
    </button>
  );

  return (
    // 13rem fits the longest English nav label ("Language models") and the
    // full product name on one line each, at the 760px minimum window width.
    <aside className="w-52 shrink-0 flex flex-col">
      {/* Header: icon + full name on the first row; the compact update control
          gets its own row under the name so it never squeezes the title into
          a truncation. The row indents past the icon to align with the name. */}
      <div className="flex flex-col gap-1.5 px-4 py-3 border-b border-border-subtle">
        <div className="flex min-w-0 items-center gap-2.5">
          <img src="./icon.png" alt="SayTypeTypeType" className="w-7 h-7 rounded-md shrink-0" />
          <span className="min-w-0 truncate text-sm font-semibold text-text tracking-[-0.01em]">
            SayTypeTypeType
          </span>
        </div>
        <UpdateButton compact className="self-start ml-[38px]" />
      </div>

      {/* Navigation */}
      <nav className="flex-1 py-3 px-2 space-y-1">
        {NAV_ITEMS.map((item) => navBtn(item.id, item.icon, t(item.label)))}
        <div className="my-2 border-t border-border-subtle" />
        <button
          type="button"
          onClick={() => {
            openHelpGuide().catch(() => showToast(t("无法打开帮助文档，请稍后重试"), "error"));
          }}
          className="w-full flex items-center gap-2.5 px-3 py-[7px] rounded-lg text-sm transition-colors text-text-dim hover:bg-fill-hover hover:text-text"
        >
          <BookOpen size={18} className="shrink-0" />
          <span className="min-w-0 whitespace-nowrap">{t("帮助说明")}</span>
        </button>
        {navBtn("about", Info, t("关于"))}
      </nav>

      <div className="p-3 border-t border-border-subtle">
        <ThemeSelector />
      </div>
      <Toast toast={toast} />
    </aside>
  );
}
