import { invoke } from "@tauri-apps/api/core";
import type {
  RemoteFileListing,
  RemoteMutationResult,
  FileTransferResult,
} from "@/types/contracts";

export const FILE_TRANSFER_EVENT = "file-transfer-event";

export const fileCommands = {
  browse: "browse_remote_files",
  upload: "upload_remote_file",
  download: "download_remote_file",
  cancel: "cancel_file_transfer",
  createFolder: "create_remote_folder",
  rename: "rename_remote_entry",
  delete: "delete_remote_entry",
} as const;

export type UploadFileInput = {
  transferId: string;
  serverId: string;
  root: string;
  localPath: string;
  remotePath: string;
  overwrite?: boolean;
};

export type DownloadFileInput = {
  transferId: string;
  serverId: string;
  root: string;
  remotePath: string;
  localPath: string;
  overwrite?: boolean;
};

export type CreateFolderInput = {
  serverId: string;
  root: string;
  parent: string;
  name: string;
};

export type RenameEntryInput = {
  serverId: string;
  root: string;
  path: string;
  newName: string;
  confirmed: boolean;
};

export type DeleteEntryInput = {
  serverId: string;
  root: string;
  path: string;
  confirmed: boolean;
  recursive?: boolean;
};

export function browseRemoteFiles(
  serverId: string,
  root: string,
  path: string,
): Promise<RemoteFileListing> {
  return invoke<RemoteFileListing>(fileCommands.browse, { id: serverId, root, path });
}

export function uploadRemoteFile(input: UploadFileInput): Promise<FileTransferResult> {
  return invoke<FileTransferResult>(fileCommands.upload, { input });
}

export function downloadRemoteFile(input: DownloadFileInput): Promise<FileTransferResult> {
  return invoke<FileTransferResult>(fileCommands.download, { input });
}

export function cancelFileTransfer(transferId: string): Promise<boolean> {
  return invoke<boolean>(fileCommands.cancel, { transferId });
}

export function createRemoteFolder(input: CreateFolderInput): Promise<RemoteMutationResult> {
  return invoke<RemoteMutationResult>(fileCommands.createFolder, { input });
}

export function renameRemoteEntry(input: RenameEntryInput): Promise<RemoteMutationResult> {
  return invoke<RemoteMutationResult>(fileCommands.rename, { input });
}

export function deleteRemoteEntry(input: DeleteEntryInput): Promise<RemoteMutationResult> {
  return invoke<RemoteMutationResult>(fileCommands.delete, { input });
}

/** Browser-safe transfer identifier usable in a remote partial filename. */
export function createTransferId(): string {
  const random = globalThis.crypto?.randomUUID?.();
  if (random) {
    return random.replaceAll("-", "");
  }
  return `transfer-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 12)}`;
}
