import { create } from "zustand";
import { getCommandErrorDetails, getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  getSystemdServices as getSystemdServicesService,
  runServiceAction as runServiceActionService,
} from "@/services/services";
import type {
  ServiceActionName,
  ServiceActionResult,
  ServiceDiscovery,
  ServiceStatus,
} from "@/types/contracts";

export type ServiceDiscoveryEntry = {
  discovery: ServiceDiscovery | null;
  pending: boolean;
  error: string | null;
  lastAttemptAt: number | null;
};

export type ServiceActionEntry = {
  pending: ServiceActionName | null;
  pendingUnit: string | null;
  result: ServiceActionResult | null;
  error: string | null;
  errorDetails: string | null;
};

type ServiceStore = {
  entries: Record<string, ServiceDiscoveryEntry>;
  actions: Record<string, ServiceActionEntry>;
  refresh: (serverId: string) => Promise<boolean>;
  runAction: (
    serverId: string,
    unit: string,
    action: ServiceActionName,
    confirmed: boolean,
  ) => Promise<boolean>;
  clearActionResult: (serverId: string) => void;
  clearServer: (serverId: string) => void;
  clearError: (serverId: string) => void;
};

function emptyEntry(): ServiceDiscoveryEntry {
  return {
    discovery: null,
    pending: false,
    error: null,
    lastAttemptAt: null,
  };
}

function emptyActionEntry(): ServiceActionEntry {
  return {
    pending: null,
    pendingUnit: null,
    result: null,
    error: null,
    errorDetails: null,
  };
}

function applyVerifiedStatus(
  discovery: ServiceDiscovery | null,
  status: ServiceStatus,
): ServiceDiscovery | null {
  if (!discovery || discovery.status !== "available") {
    return discovery;
  }

  const updatedUnit = {
    name: status.unit,
    state: status.state,
    activeState: status.activeState,
    subState: status.subState,
    loadState: status.loadState,
    description: status.description,
  };
  let found = false;
  const services = discovery.services.map((service) => {
    if (service.name !== status.unit) {
      return service;
    }
    found = true;
    return updatedUnit;
  });

  if (!found) {
    services.push(updatedUnit);
  }

  return {
    ...discovery,
    services,
    sampledAt: status.sampledAt,
    durationMs: status.durationMs,
  };
}

export const useServiceStore = create<ServiceStore>((set, get) => {
  let requestCounter = 0;
  const requestIds: Record<string, number> = {};
  let actionCounter = 0;
  const actionIds: Record<string, number> = {};

  return {
    entries: {},
    actions: {},

    refresh: async (serverId) => {
      if (!isTauriRuntime()) {
        return false;
      }

      const requestId = ++requestCounter;
      requestIds[serverId] = requestId;
      set((state) => {
        const current = state.entries[serverId] ?? emptyEntry();
        return {
          entries: {
            ...state.entries,
            [serverId]: { ...current, pending: true, error: null },
          },
        };
      });

      try {
        const discovery = await getSystemdServicesService(serverId);
        if (requestIds[serverId] !== requestId) {
          return false;
        }

        set((state) => {
          const current = state.entries[serverId] ?? emptyEntry();
          return {
            entries: {
              ...state.entries,
              [serverId]: {
                ...current,
                discovery,
                pending: false,
                error: null,
                lastAttemptAt: Date.now(),
              },
            },
          };
        });
        return true;
      } catch (error) {
        const message = getCommandErrorMessage(error);
        if (requestIds[serverId] === requestId) {
          set((state) => {
            const current = state.entries[serverId] ?? emptyEntry();
            return {
              entries: {
                ...state.entries,
                [serverId]: {
                  ...current,
                  pending: false,
                  error: message,
                  lastAttemptAt: Date.now(),
                },
              },
            };
          });
        }
        return false;
      }
    },

    runAction: async (serverId, unit, action, confirmed) => {
      if (!isTauriRuntime()) {
        return false;
      }

      const requestId = ++actionCounter;
      actionIds[serverId] = requestId;
      set((state) => {
        const current = state.actions[serverId] ?? emptyActionEntry();
        return {
          actions: {
            ...state.actions,
            [serverId]: {
              ...current,
              pending: action,
              pendingUnit: unit,
              result: null,
              error: null,
              errorDetails: null,
            },
          },
        };
      });

      try {
        const result = await runServiceActionService(serverId, unit, action, confirmed);
        if (actionIds[serverId] !== requestId) {
          return false;
        }

        set((state) => {
          const current = state.actions[serverId] ?? emptyActionEntry();
          const discoveryEntry = state.entries[serverId];
          const discovery = result.verifiedStatus
            ? applyVerifiedStatus(discoveryEntry?.discovery ?? null, result.verifiedStatus)
            : (discoveryEntry?.discovery ?? null);

          return {
            actions: {
              ...state.actions,
              [serverId]: {
                ...current,
                pending: null,
                pendingUnit: null,
                result,
                error: null,
                errorDetails: null,
              },
            },
            entries: discoveryEntry
              ? {
                  ...state.entries,
                  [serverId]: {
                    ...discoveryEntry,
                    discovery,
                    lastAttemptAt: result.verifiedStatus
                      ? Date.now()
                      : discoveryEntry.lastAttemptAt,
                  },
                }
              : state.entries,
          };
        });

        // The action result already carries the verified unit state. Re-running
        // discovery keeps every other row, the state counts, and the sample time
        // in sync when an action starts, stops, or restarts a unit.
        await get().refresh(serverId);

        return result.succeeded;
      } catch (error) {
        const message = getCommandErrorMessage(error);
        if (actionIds[serverId] === requestId) {
          set((state) => {
            const current = state.actions[serverId] ?? emptyActionEntry();
            return {
              actions: {
                ...state.actions,
                [serverId]: {
                  ...current,
                  pending: null,
                  pendingUnit: null,
                  result: null,
                  error: message,
                  errorDetails: getCommandErrorDetails(error),
                },
              },
            };
          });
        }
        return false;
      }
    },

    clearActionResult: (serverId) =>
      set((state) => {
        const actions = { ...state.actions };
        delete actions[serverId];
        return { actions };
      }),

    clearServer: (serverId) => {
      requestIds[serverId] = ++requestCounter;
      actionIds[serverId] = ++actionCounter;
      set((state) => {
        const entries = { ...state.entries };
        const actions = { ...state.actions };
        delete entries[serverId];
        delete actions[serverId];
        return { entries, actions };
      });
    },

    clearError: (serverId) =>
      set((state) => {
        const current = state.entries[serverId];
        if (!current) {
          return state;
        }

        return {
          entries: {
            ...state.entries,
            [serverId]: { ...current, error: null },
          },
        };
      }),
  };
});
