import { strict as assert } from "node:assert"
import { test } from "node:test"
import { detectLocale } from "../lib/i18n"
const cases: [string | null, string | undefined, string][] = [
  [null, undefined, "en"],
  ["", undefined, "en"],
  ["de-DE,pl;q=0.8", undefined, "en"],
  ["zh-CN,zh;q=0.9,en;q=0.8", undefined, "zh"],
  ["zh-TW", undefined, "zh"],
  ["en-GB,zh;q=0.8", undefined, "en"],
  ["zh;q=0.3,en;q=0.9", undefined, "en"],
  ["en;q=0,zh;q=0.4", undefined, "zh"],
  ["zh;q=bad,en;q=0.5", undefined, "en"],
  ["zh-CN", "en", "en"],
  ["en-US", "zh", "zh"],
  ["zh", "invalid", "zh"],
]
for (const [header, saved, expected] of cases)
  test(`locale ${header} / ${saved} -> ${expected}`, () =>
    assert.equal(detectLocale(header, saved), expected))
