/*
 * @title dsh-tauri-notification
 * @swagger 2.0
 * @version 0.0.0
 */

import type { FetchOptions } from "dsh-tauri/client";
import { ofetch } from "dsh-tauri/client";
import type * as Types from "./index.type";

export const baseURL = "/api/desktop/dsh-tauri-notification";

/** @method get */
export function getTurnEnd(params?: Types.GetTurnEndQuery, options?: FetchOptions) {
  return ofetch<Types.TurnEndResult>("/turn-end", { baseURL, method: "get", params, ...options });
}
