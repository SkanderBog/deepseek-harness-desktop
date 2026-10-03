import { cssr } from 'dsh-tauri-ui/client'
import { MOBILE_MEDIA_QUERIES } from 'dsh-tauri/client'

const { c } = cssr

export default c(`@media ${MOBILE_MEDIA_QUERIES.join(' and ')}`, [
  c('[data-dsh-mobile-preferences]', {
    display: 'flex',
    flex: '1',
    minWidth: '0',
    alignItems: 'center',
    gap: '8px',
    padding: '8px 0',
  }),
  c('[data-dsh-mobile-preferences] button', {
    marginLeft: 'auto',
    flexShrink: '0',
  }),
  c('[data-slot="conversation.composer.bar"] [class$="_dock"]', {
    display: 'none !important',
  }),
  c('[class$="_composerStack"] > [data-slot="conversation.input.dock"]', {
    display: 'none !important',
  }),
  c('[class$="_turnErrorCode"]', {
    display: 'none !important',
  }),
  c('[data-slot="conversation.header"] [class$="_header"]', {
    display: 'none !important',
  }),
  c('[data-slot="main"] header[class*="_pageHead"]', {
    paddingLeft: '0 !important',
    paddingTop: '24px !important',
  }),
  c('header[class*="_pageHead"] [class*="_toolbar"]', {
    display: 'none !important',
  }),
  c('[data-slot="conversation.view"] [class$="_scroll"]', {
    padding: '16px !important',
  }),
  c('[class*="_userStack"]', {
    maxWidth: '100% !important',
  }),
  c('[data-slot="main"] [data-conversation-scroll]', {
    paddingBottom: '0 !important',
  }),
])
