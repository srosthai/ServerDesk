import { useTauriEvent } from "@/hooks/useTauriEvent";
import { FILE_TRANSFER_EVENT } from "@/services/files";
import { useFileStore } from "@/stores/fileStore";
import type { FileTransferProgress } from "@/types/contracts";

export function useFileTransferEvents() {
  const applyProgress = useFileStore((state) => state.applyTransferProgress);

  useTauriEvent<FileTransferProgress>(FILE_TRANSFER_EVENT, applyProgress);
}
