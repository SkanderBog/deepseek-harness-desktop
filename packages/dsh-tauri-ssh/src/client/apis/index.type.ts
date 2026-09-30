export type SshConnectResponse = {
  tunnelBaseUrl?: string;
  error?: string;
};
export type SshActionResponse = {
  error?: string;
};
export type SshMachineEventsResponse = {
  items?: SshMachineEventItem[];
  nextSeq?: number;
  error?: string;
};
export type SshMachineEventItem = {
  seq?: number;
  ts?: string;
  machineId?: string;
  stage?: string;
  line?: string;
  terminal?: string;
  reason?: string;
};
export type SshMachinesResponse = {
  enabled?: boolean;
  items?: SshMachineListItem[];
  discovered?: SshMachineListItem[];
  error?: string;
};
export type SshMachineListItem = {
  id?: string;
  name?: string;
  host?: string;
  port?: number;
  user?: string;
  hasPassword?: boolean;
  hasPassphrase?: boolean;
  remotePort?: number;
  profileName?: string;
  startCommand?: string;
  color?: string;
  tintBorder?: boolean;
  state?: SshConnectionState;
  tunnelBaseUrl?: string;
  lastError?: string;
  dshMissing?: boolean;
  progress?: SshProgress;
  nextRetryAt?: number;
  authMethod?: SshAuthMethod;
};
export type SshConnectionState = "disconnected" | "testing" | "connecting" | "connected" | "reconnecting" | "given-up";
export type SshProgress = {
  phase: SshProgressPhase;
  attempt?: number;
  total?: number;
  item?: string;
  log?: string;
};
export type SshProgressPhase = "handshake" | "starting" | "probing" | "installing" | "syncing";
export type SshAuthMethod = "agent" | "key" | "password";
export type SshInstallResponse = {
  installed?: string[];
  dshRef?: string;
  dshVersion?: string;
  dshPath?: string;
  credentialsCopied?: boolean;
  credentialsError?: string;
  error?: string;
};
export type SshMachineRowBody = {
  name?: string;
  host?: string;
  port?: number;
  user?: string;
  remotePort?: number;
  profileName?: string;
  startCommand?: string;
  color?: string;
  tintBorder?: boolean;
};
export type SshMachineSecretsBody = {
  password?: string;
  passphrase?: string;
};
export type SshTestResponse = {
  ok?: boolean;
  banner?: string;
  message?: string;
  error?: string;
};
export type SshSessionRoleResponse = {
  role?: string;
  remote?: boolean;
  origin?: string;
  error?: string;
};
export type SshSettingsResponse = {
  enabled?: boolean;
  error?: string;
};
export type SyncPluginRefBody = {
  name?: string;
  spec?: string;
};
export type SyncSkillRefBody = {
  name?: string;
  root?: string;
};
export type SyncApplyResponse = {
  items?: SyncApplyItemResult[];
  error?: string;
};
export type SyncApplyItemResult = {
  kind?: string;
  name?: string;
  root?: string;
  ok?: boolean;
  error?: string;
  log?: string;
};
export type SyncPreviewResponse = {
  plugins?: SyncPreviewPluginItem[];
  skills?: SyncPreviewSkillItem[];
  error?: string;
};
export type SyncPreviewPluginItem = {
  name?: string;
  spec?: string;
  syncable?: boolean;
  reason?: string;
};
export type SyncPreviewSkillItem = {
  name?: string;
  root?: string;
};

export interface SshMachineIdBody {
  machineId?: string;
}
export interface SshMachineSaveBody {
  machineId?: string;
  row?: SshMachineRowBody;
  secrets?: SshMachineSecretsBody;
}
export interface SshSettingsBody {
  enabled?: boolean;
}
export interface SyncApplyBody {
  machineId?: string;
  plugins?: SyncPluginRefBody[];
  skills?: SyncSkillRefBody[];
}
export interface GetMachinesEventsQuery {
  machineId?: string;
  sinceSeq?: number;
}
