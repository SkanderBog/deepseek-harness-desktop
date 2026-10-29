export interface PreinstallPlugin {
  id: string
  name: string
  description: string
  repo_url: string
  recommended: boolean
  fix: boolean
  defaultChecked: boolean
  defaultUnchecked: boolean
  installed: boolean
  unsupported: boolean
}

export interface PreinstallLogPayload {
  line: string
}

export interface PreinstallSelection {
  installIds?: string[]
  uninstallIds?: string[]
}

export interface IncompatibleVersion {
  name: string
  version: string
  runtime_version: string
}

export interface PolicyBlockedVersion {
  name: string
  version: string
}
