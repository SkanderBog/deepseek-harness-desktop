/*
 * @title dsh-tauri-ssh
 * @swagger 2.0
 * @version 0.0.0
 */

import type { FetchOptions } from "./http";
import { ofetch } from "./http";
import type * as Types from "./remote.types";

export const baseURL = "/api/desktop/dsh-tauri-ssh";

/** @method post */
export function postMachinesConnect(body: Types.SshMachineIdBody, options?: FetchOptions) {
  return ofetch<Types.SshConnectResponse>("/machines/connect", { baseURL, method: "post", body, ...options });
}

/** @method get */
export function getMachines(options?: FetchOptions) {
  return ofetch<Types.SshMachinesResponse>("/machines", { baseURL, method: "get", ...options });
}

/** @method post */
export function postMachines(body: Types.SshMachineSaveBody, options?: FetchOptions) {
  return ofetch<Types.SshActionResponse>("/machines", { baseURL, method: "post", body, ...options });
}

/** @method delete */
export function deleteMachines(body: Types.SshMachineIdBody, options?: FetchOptions) {
  return ofetch<Types.SshActionResponse>("/machines", { baseURL, method: "delete", body, ...options });
}

/** @method post */
export function postMachinesDisconnect(body: Types.SshMachineIdBody, options?: FetchOptions) {
  return ofetch<Types.SshActionResponse>("/machines/disconnect", { baseURL, method: "post", body, ...options });
}

/** @method get */
export function getMachinesEvents(params?: Types.GetMachinesEventsQuery, options?: FetchOptions) {
  return ofetch<Types.SshMachineEventsResponse>("/machines/events", { baseURL, method: "get", params, ...options });
}

/** @method post */
export function postMachinesInstall(body: Types.SshMachineIdBody, options?: FetchOptions) {
  return ofetch<Types.SshInstallResponse>("/machines/install", { baseURL, method: "post", body, ...options });
}

/** @method post */
export function postMachinesTest(body: Types.SshMachineIdBody, options?: FetchOptions) {
  return ofetch<Types.SshTestResponse>("/machines/test", { baseURL, method: "post", body, ...options });
}

/** @method get */
export function getSessionRole(options?: FetchOptions) {
  return ofetch<Types.SshSessionRoleResponse>("/session/role", { baseURL, method: "get", ...options });
}

/** @method get */
export function getSettings(options?: FetchOptions) {
  return ofetch<Types.SshSettingsResponse>("/settings", { baseURL, method: "get", ...options });
}

/** @method post */
export function postSettings(body: Types.SshSettingsBody, options?: FetchOptions) {
  return ofetch<Types.SshSettingsResponse>("/settings", { baseURL, method: "post", body, ...options });
}

/** @method post */
export function postSyncApply(body: Types.SyncApplyBody, options?: FetchOptions) {
  return ofetch<Types.SyncApplyResponse>("/sync/apply", { baseURL, method: "post", body, ...options });
}

/** @method get */
export function getSyncPreview(options?: FetchOptions) {
  return ofetch<Types.SyncPreviewResponse>("/sync/preview", { baseURL, method: "get", ...options });
}
