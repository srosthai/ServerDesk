import { open, save } from "@tauri-apps/plugin-dialog";
import { isTauriRuntime } from "@/lib/tauri";

export type FileDialogFilter = {
  name: string;
  extensions: string[];
};

/**
 * Opens the native file picker for a local source file. The web preview has no
 * native picker, so the caller keeps its text field as a fallback.
 */
export async function pickLocalFile({
  title,
  filters,
}: {
  title: string;
  filters: FileDialogFilter[];
}): Promise<string | null> {
  if (!isTauriRuntime()) {
    return null;
  }
  const selected = await open({
    title,
    multiple: false,
    directory: false,
    filters,
  });
  return typeof selected === "string" ? selected : null;
}

/**
 * Opens the native save dialog for a local destination file. Remote paths are
 * always typed because the desktop process cannot browse the server.
 */
export async function pickLocalSavePath({
  title,
  defaultPath,
  filters,
}: {
  title: string;
  defaultPath?: string;
  filters: FileDialogFilter[];
}): Promise<string | null> {
  if (!isTauriRuntime()) {
    return null;
  }
  return save({
    title,
    defaultPath,
    filters,
  });
}
