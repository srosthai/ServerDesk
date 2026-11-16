import {
  useDatabaseWorkspaceStore,
  type DatabaseWorkspaceTaskStatus,
} from "@/stores/databaseWorkspaceStore";

export function beginOperationTask({
  profileId,
  id,
  label,
  detail,
}: {
  profileId: string;
  id: string;
  label: string;
  detail?: string;
}) {
  useDatabaseWorkspaceStore.getState().addTask(profileId, {
    id,
    label,
    status: "running",
    detail: detail ?? "Preparing the transfer",
    startedAt: Date.now(),
  });
}

export function updateOperationTask({
  profileId,
  id,
  status,
  detail,
  partial,
}: {
  profileId: string;
  id: string;
  status: DatabaseWorkspaceTaskStatus;
  detail?: string;
  partial?: boolean;
}) {
  useDatabaseWorkspaceStore.getState().updateTask(profileId, id, {
    status,
    detail,
    partial,
  });
}

export function dismissOperationTask(profileId: string, id: string) {
  useDatabaseWorkspaceStore.getState().removeTask(profileId, id);
}
