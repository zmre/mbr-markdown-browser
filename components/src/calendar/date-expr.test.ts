import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { EMPTY_RANGE, evaluateDateExpr, isEmptyRange, parseDateExpr, rangeContains } from './date-expr.js'
import { addDays } from './note-date.js'

interface Case {
  expr: string
  today?: string
  invalid?: boolean
  empty?: boolean
  from?: string | null
  to?: string | null
}

/**
 * The table `crates/mbr-core/src/date_expr.rs` also reads: both sides of the
 * grammar answer every case identically. (vitest runs from `components/`.)
 */
const table = JSON.parse(
  readFileSync(join(process.cwd(), '..', 'tests', 'fixtures', 'date-expr-cases.json'), 'utf8')
) as { today: string; cases: Case[] }

describe('date expressions (shared table with Rust)', () => {
  it('has the full table', () => {
    expect(table.cases.length).toBeGreaterThan(40)
  })

  for (const c of table.cases) {
    const today = c.today ?? table.today
    it(`${JSON.stringify(c.expr)} today ${today}`, () => {
      const got = evaluateDateExpr(c.expr, today)
      if (c.invalid) {
        expect(got).toBeNull()
      } else if (c.empty) {
        expect(got).toEqual(EMPTY_RANGE)
        expect(isEmptyRange(got!)).toBe(true)
        for (const day of ['1000-01-01', '2026-11-06', '9999-12-31']) expect(rangeContains(got!, day), day).toBe(false)
      } else {
        expect(got).toEqual({ from: c.from ?? null, to: c.to ?? null })
      }
    })
  }
})

/** A small deterministic generator (no extra dependency for property tests). */
function rng(seed: number): () => number {
  let s = seed >>> 0
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0
    return s / 2 ** 32
  }
}

const randomDay = (next: () => number): string => addDays('1000-01-01', Math.floor(next() * 3_000_000) % 3_287_181)

describe('date expression invariants', () => {
  it('a day key is exactly that day', () => {
    const next = rng(1)
    for (let i = 0; i < 500; i++) {
      const day = randomDay(next)
      expect(evaluateDateExpr(day, '2026-11-06')).toEqual({ from: day, to: day })
    }
  })

  it('every generated expression has from <= to', () => {
    const next = rng(2)
    const pick = <T,>(xs: readonly T[]): T => xs[Math.floor(next() * xs.length)]
    const rel = ['today', 'yesterday', 'this-week', 'last-week', 'this-month', 'last-month', 'this-year']
    for (let i = 0; i < 2000; i++) {
      const a = randomDay(next)
      const b = randomDay(next)
      const shape = pick(['a', 'a..b', 'cmp', 'rel', 'count', 'month', 'year'] as const)
      const expr =
        shape === 'a'
          ? a
          : shape === 'a..b'
            ? `${a}..${b}`
            : shape === 'cmp'
              ? `${pick(['>=', '>', '<=', '<'])}${a}`
              : shape === 'rel'
                ? pick(rel)
                : shape === 'count'
                  ? `-${1 + Math.floor(next() * 999_999)}${pick(['d', 'w', 'm', 'y'])}`
                  : shape === 'month'
                    ? a.slice(0, 7)
                    : a.slice(0, 4)
      const range = evaluateDateExpr(expr, b)
      expect(range, expr).not.toBeNull()
      if (range && isEmptyRange(range)) {
        expect(range, expr).toEqual(EMPTY_RANGE)
        continue
      }
      if (range?.from && range.to) expect(range.from <= range.to, `${expr}: ${range.from} > ${range.to}`).toBe(true)
      for (const key of [range?.from, range?.to]) if (key) expect(key, expr).toMatch(/^\d{4}-\d{2}-\d{2}$/)
    }
  })

  it('never throws on noise', () => {
    const next = rng(3)
    const alphabet = '-.<>=0123456789WwQqdmy:ab '
    for (let i = 0; i < 5000; i++) {
      const len = Math.floor(next() * 24)
      let s = ''
      for (let j = 0; j < len; j++) s += alphabet[Math.floor(next() * alphabet.length)]
      expect(() => evaluateDateExpr(s, '2026-11-06')).not.toThrow()
    }
  })

  it('parseDateExpr rejects what is not an expression', () => {
    expect(parseDateExpr('draft')).toBeNull()
    expect(parseDateExpr('..')).toBeNull()
  })

  it('rangeContains is inclusive with open ends', () => {
    const r = { from: '2026-10-01', to: '2026-10-31' }
    expect(rangeContains(r, '2026-10-01')).toBe(true)
    expect(rangeContains(r, '2026-10-31')).toBe(true)
    expect(rangeContains(r, '2026-11-01')).toBe(false)
    expect(rangeContains({ from: null, to: null }, '1970-01-01')).toBe(true)
  })
})
