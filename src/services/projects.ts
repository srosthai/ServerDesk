import { invoke } from "@tauri-apps/api/core";
import type {
  ArtisanActionName,
  ProjectArtisanRun,
  ProjectRecord,
  ProjectScanResult,
  RemoteDirectoryListing,
} from "@/types/contracts";

export const projectCommands = {
  list: "list_projects",
  browse: "browse_remote_directories",
  scan: "scan_remote_projects",
  action: "run_project_artisan_action",
  delete: "delete_project",
} as const;

export function listProjects(serverId: string): Promise<ProjectRecord[]> {
  return invoke<ProjectRecord[]>(projectCommands.list, { id: serverId });
}

export function browseRemoteDirectories(
  serverId: string,
  path: string,
): Promise<RemoteDirectoryListing> {
  return invoke<RemoteDirectoryListing>(projectCommands.browse, { id: serverId, path });
}

export function scanRemoteProjects(
  serverId: string,
  path: string,
  includeChildren: boolean,
): Promise<ProjectScanResult> {
  return invoke<ProjectScanResult>(projectCommands.scan, {
    id: serverId,
    path,
    includeChildren,
  });
}

export function runProjectArtisanAction(
  serverId: string,
  projectId: string,
  action: ArtisanActionName,
  confirmed: boolean,
): Promise<ProjectArtisanRun> {
  return invoke<ProjectArtisanRun>(projectCommands.action, {
    id: serverId,
    projectId,
    action,
    confirmed,
  });
}

export function deleteProject(projectId: string): Promise<void> {
  return invoke<void>(projectCommands.delete, { id: projectId, confirmed: true });
}
