import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const nextVersion = process.argv[2]
const semverPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/

if (!nextVersion || !semverPattern.test(nextVersion)) {
  console.error('Usage: npm run version:set -- <version>')
  console.error('Example: npm run version:set -- 0.1.1')
  console.error('Use a semantic version without a leading "v".')
  process.exit(1)
}

const paths = {
  packageJson: path.join(repositoryRoot, 'package.json'),
  packageLock: path.join(repositoryRoot, 'package-lock.json'),
  cargoToml: path.join(repositoryRoot, 'src-tauri', 'Cargo.toml'),
  cargoLock: path.join(repositoryRoot, 'src-tauri', 'Cargo.lock'),
  tauriConfig: path.join(repositoryRoot, 'src-tauri', 'tauri.conf.json'),
}

const source = Object.fromEntries(
  Object.entries(paths).map(([name, filePath]) => [name, fs.readFileSync(filePath, 'utf8')]),
)

const packageJson = JSON.parse(source.packageJson)
const packageLock = JSON.parse(source.packageLock)
const tauriConfig = JSON.parse(source.tauriConfig)

function replacePackageVersion(toml, sectionHeader, packageName, version) {
  const escapedName = packageName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const sectionPattern = new RegExp(
    `(\\[${sectionHeader}\\](?:(?!\\n\\[)[\\s\\S])*?\\nname\\s*=\\s*"${escapedName}"(?:(?!\\n\\[)[\\s\\S])*?\\nversion\\s*=\\s*")[^"]+(")`,
  )

  if (!sectionPattern.test(toml)) {
    throw new Error(`Could not find ${packageName} version in ${sectionHeader}`)
  }

  return toml.replace(sectionPattern, `$1${version}$2`)
}

function readPackageVersion(toml, sectionHeader, packageName) {
  const escapedName = packageName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const sectionPattern = new RegExp(
    `\\[${sectionHeader}\\](?:(?!\\n\\[)[\\s\\S])*?\\nname\\s*=\\s*"${escapedName}"(?:(?!\\n\\[)[\\s\\S])*?\\nversion\\s*=\\s*"([^"]+)"`,
  )
  const match = toml.match(sectionPattern)

  if (!match) {
    throw new Error(`Could not read ${packageName} version in ${sectionHeader}`)
  }

  return match[1]
}

function replaceManifestVersion(toml, version) {
  const packageSectionPattern = /(\[package\](?:(?!\n\[)[\s\S])*?\nversion\s*=\s*")[^"]+(")/
  if (!packageSectionPattern.test(toml)) {
    throw new Error('Could not find package version in src-tauri/Cargo.toml')
  }
  return toml.replace(packageSectionPattern, `$1${version}$2`)
}

function readManifestVersion(toml) {
  const match = toml.match(/\[package\](?:(?!\n\[)[\s\S])*?\nversion\s*=\s*"([^"]+)"/)
  if (!match) {
    throw new Error('Could not read package version from src-tauri/Cargo.toml')
  }
  return match[1]
}

function stringifyJsonLike(original, value) {
  const eol = original.includes('\r\n') ? '\r\n' : '\n'
  return `${JSON.stringify(value, null, 2).replaceAll('\n', eol)}${eol}`
}

const currentVersions = {
  'package.json': packageJson.version,
  'package-lock.json': packageLock.version,
  'package-lock.json root package': packageLock.packages?.['']?.version,
  'src-tauri/Cargo.toml': readManifestVersion(source.cargoToml),
  'src-tauri/Cargo.lock': readPackageVersion(source.cargoLock, '\\[package\\]', 'arka'),
  'src-tauri/tauri.conf.json': tauriConfig.version,
}

const distinctCurrentVersions = new Set(Object.values(currentVersions))
if (distinctCurrentVersions.size !== 1) {
  console.error('Version files are already out of sync:')
  for (const [file, version] of Object.entries(currentVersions)) {
    console.error(`- ${file}: ${version ?? '<missing>'}`)
  }
  process.exit(1)
}

packageJson.version = nextVersion
packageLock.version = nextVersion
packageLock.packages[''].version = nextVersion
tauriConfig.version = nextVersion

const updated = {
  packageJson: stringifyJsonLike(source.packageJson, packageJson),
  packageLock: stringifyJsonLike(source.packageLock, packageLock),
  cargoToml: replaceManifestVersion(source.cargoToml, nextVersion),
  cargoLock: replacePackageVersion(source.cargoLock, '\\[package\\]', 'arka', nextVersion),
  tauriConfig: stringifyJsonLike(source.tauriConfig, tauriConfig),
}

for (const [name, filePath] of Object.entries(paths)) {
  fs.writeFileSync(filePath, updated[name], 'utf8')
}

console.log(`Updated ARKA version to ${nextVersion}:`)
for (const filePath of Object.values(paths)) {
  console.log(`- ${path.relative(repositoryRoot, filePath)}`)
}
