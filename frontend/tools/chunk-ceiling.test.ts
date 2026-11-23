import { describe, expect, it } from 'vitest'
import { overCeiling } from './chunk-ceiling'

const chunk = (fileName: string, kb: number) => ({ type: 'chunk' as const, fileName, code: 'x'.repeat(kb * 1000) })

describe('overCeiling', () => {
  it('names a script file over the ceiling, with its size', () => {
    expect(overCeiling([chunk('index.js', 501)], 500)).toEqual([{ fileName: 'index.js', kb: 501 }])
  })

  it('lets a file at the ceiling through: Vite warns only above it', () => {
    expect(overCeiling([chunk('index.js', 500)], 500)).toEqual([])
  })

  it('does not count styles and images, which are assets', () => {
    expect(overCeiling([{ type: 'asset', fileName: 'index.css' }, chunk('index.js', 10)], 5)).toEqual([{ fileName: 'index.js', kb: 10 }])
  })

  it('lists every file over it, largest first', () => {
    const files = [chunk('a.js', 600), chunk('b.js', 100), chunk('c.js', 900)]
    expect(overCeiling(files, 500).map((file) => file.fileName)).toEqual(['c.js', 'a.js'])
  })
})
