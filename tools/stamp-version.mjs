// Writes the manifest's version into the transcripts and examples that show
// it as a bare number - the health answer in the README, the build-info
// metric, and the version an installation pins and upgrades to. furca's bump
// rewrites `v1.2.3` and leaves a bare `1.2.3` alone, because in prose it is as
// often a dependency's number; these are the product's, so they are stamped.
//
//   node tools/stamp-version.mjs
//
// `furca release run` runs it after the bump (`[bump] after` in release.toml),
// and tests/release_consistency.rs holds the README to the manifest.
import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const manifest = readFileSync(join(root, 'Cargo.toml'), 'utf8')
const version = manifest.match(/^version = "(\d+\.\d+\.\d+)"$/m)?.[1]
if (!version) {
  console.error('stamp-version: no version in Cargo.toml')
  process.exit(1)
}

const NUMBER = String.raw`\d+\.\d+\.\d+`
// Each place, and what it looks like: the number is the only part replaced.
const places = [
  ['README.md', new RegExp(String.raw`("version":")${NUMBER}(")`, 'g')],
  ['docs/src/content/docs/reference/server-metrics.md', new RegExp(String.raw`(kasl_build_info\{version=")${NUMBER}(")`, 'g')],
  ['docs/src/content/docs/guides/installing-it.md', new RegExp(String.raw`(KASL_VERSION=)${NUMBER}()`, 'g')],
  ['docs/src/content/docs/guides/installing-it.md', new RegExp(String.raw`(upgrade\.sh )${NUMBER}()`, 'g')],
]

for (const [file, pattern] of places) {
  const path = join(root, file)
  const before = readFileSync(path, 'utf8')
  if (!pattern.test(before)) {
    // A place that stopped matching is a transcript that will go stale in
    // silence: said, and the run stopped.
    console.error(`stamp-version: ${file} no longer shows ${pattern.source}`)
    process.exit(1)
  }
  pattern.lastIndex = 0
  const after = before.replace(pattern, `$1${version}$2`)
  if (after !== before) writeFileSync(path, after)
}
console.log(`stamp-version: ${version}`)
