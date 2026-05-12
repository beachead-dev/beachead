/**
 * Pure function that derives the enabled/disabled state of container action buttons
 * based on the container's current status and whether it is unmanaged.
 *
 * Rules:
 * - "running" + managed → Stop enabled, Start/Remove disabled
 * - "stopped"/"created" + managed → Start/Remove enabled, Stop disabled
 * - "running" + unmanaged → Stop/Remove enabled, Start disabled
 * - "stopped"/"created" + unmanaged → Remove only
 * - anything else → all disabled
 */
export interface ContainerButtonStates {
  startEnabled: boolean;
  stopEnabled: boolean;
  removeEnabled: boolean;
}

export function deriveContainerButtonStates(
  status: string | null,
  isUnmanaged: boolean,
): ContainerButtonStates {
  if (isUnmanaged) {
    switch (status) {
      case "running":
        return { startEnabled: false, stopEnabled: true, removeEnabled: true };
      case "stopped":
      case "exited":
      case "created":
        return { startEnabled: false, stopEnabled: false, removeEnabled: true };
      default:
        return { startEnabled: false, stopEnabled: false, removeEnabled: false };
    }
  }

  switch (status) {
    case "running":
      return { startEnabled: false, stopEnabled: true, removeEnabled: false };
    case "stopped":
    case "exited":
    case "created":
      return { startEnabled: true, stopEnabled: false, removeEnabled: true };
    default:
      return { startEnabled: false, stopEnabled: false, removeEnabled: false };
  }
}
