import { invoke } from "@tauri-apps/api/core";
import type { RemoteFileListing } from "@/types/contracts";

export const fileCommands = {
  browse: "browse_remote_files",
} as const;

export function browseRemoteFiles(
  serverId: string,
  root: string,
  path: string,
): Promise<RemoteFileListing> {
  return invoke<RemoteFileListing>(fileCommands.browse, { id: serverId, root, path });
}
