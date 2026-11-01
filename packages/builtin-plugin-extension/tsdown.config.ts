import { defineDshConfig } from '../builtin-plugin-config/src/index.ts'

// Client-only libraries must be bundled into the classic loader artifact. The
// DSH client module table only materializes platform seed words and declared
// client injections, so leaving these imports external makes runtime require()
// fail even though the packages exist in the host profile.
export default defineDshConfig({
  server: {
    noExternal: ['smol-toml', 'yaml'],
  },
  client: {
    noExternal: ['dompurify', 'marked'],
  },
})
