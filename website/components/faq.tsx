"use client"
import Link from "next/link"
import {
  Accordion,
  AccordionItem,
  AccordionTrigger,
  AccordionContent,
} from "@/components/ui/accordion"
import items from "@/lib/faq.json"
import type { Locale } from "@/lib/i18n"
export function FAQ({ locale }: { locale: Locale }) {
  return (
    <Accordion defaultValue={[0]} className="gap-2">
      {items.map((item, index) => (
        <AccordionItem
          key={index}
          value={index}
          className="rounded-lg border-0 bg-muted px-6 not-last:border-0"
        >
          <AccordionTrigger className="min-h-16 py-5 text-base">
            {item.question[locale]}
          </AccordionTrigger>
          <AccordionContent className="pb-6 text-base leading-relaxed">
            {item.answer[locale]}
            {item.link && (
              <Link
                href={
                  item.link.path.startsWith("/")
                    ? `/${locale}${item.link.path}`
                    : item.link.path
                }
                className="mt-3 block w-fit underline underline-offset-4"
              >
                {item.link.label[locale]}
              </Link>
            )}
          </AccordionContent>
        </AccordionItem>
      ))}
    </Accordion>
  )
}
