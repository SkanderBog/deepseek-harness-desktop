export type SessionResumeResponse = {
  ok?: boolean;
  error?: string;
};
export type UngroupedResponse = {
  cwd?: string;
  error?: string;
};

export interface PostSessionResumeBody {
  sessionId?: string;
  recoverFromSessionId?: string;
}
