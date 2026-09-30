import type { LiveSnapshot as ApiLiveSnapshot, TurnFileChange as ApiTurnFileChange, SummaryPayload } from '../apis/index.type'

export type { TurnFileStatus } from '../apis/index.type'

export interface TurnFileChange extends ApiTurnFileChange {}

export interface TurnSummary extends Omit<SummaryPayload['turns'][number], 'hasBaseline'> {
  hasBaseline?: boolean
}

export interface SessionSummary extends Omit<SummaryPayload, 'turns'> {
  turns: TurnSummary[]
}

export interface LiveSnapshot extends ApiLiveSnapshot {}

export type LocaleKey
  = | 'runningChanged'
    | 'binary'
    | 'pasteChip'
    | 'pasteChipTitled'
