import { invoke } from "@tauri-apps/api/core";
import type {
  OperationHistoryEntry,
  OperationHistoryFilter,
  OperationHistoryPolicy,
} from "@/types/contracts";

export const historyCommands = {
  list: "list_operation_history",
  policy: "get_operation_history_policy",
  clear: "clear_operation_history",
} as const;

export function listOperationHistory(
  filter: OperationHistoryFilter = {},
): Promise<OperationHistoryEntry[]> {
  return invoke<OperationHistoryEntry[]>(historyCommands.list, { filter });
}

export function getOperationHistoryPolicy(): Promise<OperationHistoryPolicy> {
  return invoke<OperationHistoryPolicy>(historyCommands.policy);
}

export function clearOperationHistory(confirmed: boolean): Promise<void> {
  return invoke<void>(historyCommands.clear, { confirmed });
}
