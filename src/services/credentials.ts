import { invoke } from "@tauri-apps/api/core";

export const credentialCommands = {
  create: "create_credential",
  store: "store_credential",
  delete: "delete_credential",
  createPrivateKey: "create_private_key_credential",
  storePrivateKey: "store_private_key_credential",
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

export function createPrivateKeyCredential(
  key: string,
  passphrase: string | null,
): Promise<string> {
  return invoke<string>(credentialCommands.createPrivateKey, { key, passphrase });
}

export function storePrivateKeyCredential(
  reference: string,
  key: string,
  passphrase: string | null,
): Promise<void> {
  return invoke<void>(credentialCommands.storePrivateKey, { reference, key, passphrase });
}
