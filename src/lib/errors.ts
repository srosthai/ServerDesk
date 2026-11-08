import type { CommandError } from "@/types/contracts";

export function isCommandError(error: unknown): error is CommandError {
  if (typeof error !== "object" || error === null) {
    return false;
  }

  return (
    "code" in error &&
    typeof error.code === "string" &&
    "message" in error &&
    typeof error.message === "string"
  );
}

export function getCommandErrorMessage(error: unknown): string {
  if (isCommandError(error)) {
    return error.message;
  }

  if (typeof error === "string" && error.trim().length > 0) {
    return error;
  }

  return "The operation failed.";
}
