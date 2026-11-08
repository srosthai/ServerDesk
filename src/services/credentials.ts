import { invoke } from "@tauri-apps/api/core";

export const credentialCommands = {
  create: "create_credential",
  store: "store_credential",
  delete: "delete_credential",
} as const;

export function createCredential(secret: string): Promise<string> {
  return invoke<string>(credentialCommands.create, { secret });
}

export function storeCredential(reference: string, secret: string): Promise<void> {
  return invoke<void>(credentialCommands.store, { reference, secret });
}

export function deleteCredential(reference: string): Promise<void> {
  return invoke<void>(credentialCommands.delete, { reference });
}
