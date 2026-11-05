/**
 * Root for all setting pages
 * Use Page* as Page Layout
 * ┌-- PageLayout --┐
 * | PageHeader     |
 * | PageContent    |
 * └----------------┘
 *
 * Use Section* as grouped configuration Layout
 * ┌- Section -----------------┐
 * | SectionHeader             |
 * | ┌- SectionContent ------┐ |
 * | | ┌- SectionItemList -┐ | |
 * | | | ┌- SectionItem -┐ | | |
 * | | | | children      | | | |
 * | | | └---------------┘ | | |
 * | | └-------------------┘ | |
 * | |-----------------------┘ |
 * └---------------------------┘
 */

import { Info } from "lucide-react";
import type { ReactNode } from "react";
import { useT } from "@/i18n";
import { Tooltip } from "@/settings/components/Tooltip";

export function PageLayout({ children }: { children: ReactNode }) {
  return <div className="space-y-5">{children}</div>;
}

/* ---------- PageHeader ---------- */

export function PageHeader({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children?: ReactNode;
}) {
  return (
    <div className="mb-6">
      <h1 className="text-xl font-semibold tracking-[-0.01em] text-text">{title}</h1>
      {description && <p className="text-sm text-text-muted mt-1">{description}</p>}
      {children}
    </div>
  );
}

/* ---------- PageContent — free-form content area ---------- */

export function PageContent({ children }: { children: ReactNode }) {
  return <div>{children}</div>;
}

/* ========================================================================
   Section — Card-like container grouping related settings.
   Replaces the old .section-card / Card component.
   ======================================================================== */

export function Section({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={`bg-surface-card py-2 space-y-4 border border-border rounded-xl shadow-[0_1px_3px_rgba(0,0,0,0.08)] ${className}`}
    >
      {children}
    </div>
  );
}

/* ---------- SectionHeader — title row inside a Section ---------- */

export function SectionHeader({
  title,
  subtitle,
  action,
}: {
  title: string;
  subtitle?: string;
  action?: ReactNode;
}) {
  return (
    <div className="px-4 pt-2 flex justify-between gap-2 w-full items-center">
      <div className="space-y-1">
        <h2 className="text-sm font-semibold">{title}</h2>
        {subtitle && <p className="min-w-0 flex-1 text-sm text-text-muted mt-0.5 ">{subtitle}</p>}
      </div>

      {action && (
        <div className={`shrink-0 flex justify-end max-w-[55%] ${subtitle ? "mb-auto" : ""}`}>
          {action}
        </div>
      )}
    </div>
  );
}

/* ---------- SectionContent — padded body inside a Section ---------- */

export function SectionContent({
  children,
  className = "",
}: {
  children: ReactNode;
  className?: string;
}) {
  return <div className={`px-4 space-y-2 ${className}`}>{children}</div>;
}

/* ---------- SectionItemList — vertical list of items with 4px top/bottom padding ---------- */

export function SectionItemList({
  children,
  className = "",
}: {
  children: ReactNode;
  className?: string;
}) {
  return <div className={`space-y-2 ${className}`}>{children}</div>;
}

/* ---------- SectionItem — title/action row with optional content below ---------- */

/**
 * Label column width for rows that need pixel-identical input columns
 * (`labelClassName` on `SectionItem`).
 *
 * A percentage — not a fixed pixel width — keeps it responsive: the input
 * column always receives the remaining share of the row at every supported
 * window size, and every row that opts into this column starts and ends its
 * control at exactly the same x, so a long `description` in one row can never
 * move or shrink the controls of the rows below it.
 */
export const SETTING_LABEL_COLUMN = "w-[32%]";

export function SectionItem({
  title,
  description,
  tooltip,
  action,
  children,
  last = false,
  className = "flex-1",
  labelClassName,
}: {
  title: string;
  description?: ReactNode;
  tooltip?: ReactNode;
  action?: ReactNode;
  children?: ReactNode;
  last?: boolean;
  className?: string;
  /**
   * Width class for the label column, e.g. `SETTING_LABEL_COLUMN`.
   *
   * Without it the label column is content-sized: `flex-1` on the action side
   * has a zero flex-basis, so it carries no shrink weight and the label absorbs
   * every pixel of shrink — a long `description` then squeezes the control down
   * to its intrinsic minimum width. Passing a fixed label column makes the
   * control column content-independent (and identical across rows), which is
   * what text inputs in a form column need.
   */
  labelClassName?: string;
}) {
  const t = useT();
  // The label column is measured, never content-driven, when a column class is
  // given; `min-w-0` on the control column lets the input fill it (and shrink
  // with it) instead of being clamped at its intrinsic width.
  const labelLayout = labelClassName ? `${labelClassName} shrink-0 min-w-0` : "min-w-0";
  const actionLayout = labelClassName ? `min-w-0 ${className}` : `max-w-3/4 ${className}`;
  return (
    <div
      className={`group flex flex-col gap-2 min-h-10 justify-center ${
        last ? "" : "border-b border-border-subtle pb-2"
      }`}
    >
      <div className="flex justify-between w-full gap-2 items-center">
        <div className={labelLayout}>
          <div className="flex items-center gap-1">
            <span>{title}</span>
            {tooltip && (
              <Tooltip content={tooltip} label={t("{title}说明", { title })}>
                <Info size={14} aria-hidden="true" />
              </Tooltip>
            )}
          </div>
          {description && (
            <div className="text-xs text-text-muted text-wrap break-words">{description}</div>
          )}
        </div>
        {action && (
          <div className={`flex justify-end ${actionLayout} ${description ? "mb-auto" : ""}`}>
            {action}
          </div>
        )}
      </div>

      {children && <div className="w-full min-w-0">{children}</div>}
    </div>
  );
}
