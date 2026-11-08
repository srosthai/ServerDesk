import { invoke } from "@tauri-apps/api/core";
import type { ConnectionSnapshot, TrustedHostKey } from "@/types/contracts";

export const knownHostCommands = {
  list: "list_known_hosts",
  trust: "trust_host_key",
  reject: "reject_host_key",
  remove: "delete_known_host",
} as const;

export function listKnownHosts(serverId: string): Promise<TrustedHostKey[]> {
  return invoke<TrustedHostKey[]>(knownHostCommands.list, { id: serverId });
}

export function trustHostKey(
  serverId: string,
  keyType: string,
  fingerprint: string,
): Promise<TrustedHostKey> {
  return invoke<TrustedHostKey>(knownHostCommands.trust, {
    id: serverId,
    keyType,
    fingerprint,
  });
}

export function rejectHostKey(
  serverId: string,
  keyType: string,
  fingerprint: string,
): Promise<ConnectionSnapshot> {
  return invoke<ConnectionSnapshot>(knownHostCommands.reject, {
    id: serverId,
    keyType,
    fingerprint,
  });
}

export function deleteKnownHost(serverId: string, keyType: string): Promise<void> {
  return invoke<void>(knownHostCommands.remove, {
    id: serverId,
    keyType,
  });
}
