import type { Plugin } from 'vite'

/** The size of a script file the build refuses, in kB - Vite's own warning
 * line, so the two cannot disagree. */
export const CHUNK_LIMIT_KB = 500

/** A file of the bundle, as far as the ceiling needs to know. */
interface BundleFile {
  type: 'chunk' | 'asset'
  fileName: string
  code?: string
}

/**
 * The script files of a bundle over the ceiling, largest first, each with its
 * size in kB.
 *
 * Measured the way Vite measures for its warning - the minified code's
 * length over a thousand - so a build that passes here is one Vite says
 * nothing about. Styles and images are assets and are not counted: the
 * ceiling is about what a browser has to parse before it can draw.
 */
export function overCeiling(files: BundleFile[], limitKb: number = CHUNK_LIMIT_KB): { fileName: string; kb: number }[] {
  return files
    .filter((file) => file.type === 'chunk' && file.code !== undefined)
    .map((file) => ({ fileName: file.fileName, kb: (file.code?.length ?? 0) / 1000 }))
    .filter((file) => file.kb > limitKb)
    .sort((left, right) => right.kb - left.kb)
}

/**
 * Fails the build where Vite only warns.
 *
 * The warning came up release after release and was read past each time,
 * while the page an employee's phone opens grew by forty kB a version (wish
 * 2583). Splitting the screens by route brought it back under; this keeps it
 * there, in CI, where a warning nobody reads becomes a red build somebody
 * has to answer.
 */
export function chunkCeiling(limitKb: number = CHUNK_LIMIT_KB): Plugin {
  return {
    name: 'kasl-chunk-ceiling',
    apply: 'build',
    generateBundle(_options, bundle) {
      const over = overCeiling(Object.values(bundle) as BundleFile[], limitKb)
      if (over.length === 0) return
      const list = over.map((file) => `${file.fileName} (${file.kb.toFixed(1)} kB)`).join(', ')
      this.error(
        `script files over ${limitKb} kB: ${list}. Split the code that pulls them in - a screen is lazy in App.tsx - rather than raising the ceiling.`,
      )
    },
  }
}
