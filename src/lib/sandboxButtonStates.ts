/**
 * Pure function that derives the enabled/disabled state of sandbox action buttons
 * based on the sandbox's current status.
 *
 * Rules:
 * - "running" → Stop enabled, Start/Remove disabled
 * - "stopped" → Start/Remove enabled, Stop disabled
 * - anything else (null, empty, unknown) → all disabled
 */
export interface SandboxButtonStates {
  startEnabled: boolean;
  stopEnabled: boolean;
  removeEnabled: boolean;
}

export function deriveSandboxButtonStates(
  status: string | null,
): SandboxButtonStates {
  switch (status) {
    case "running":
      return { startEnabled: false, stopEnabled: true, removeEnabled: false };
    case "stopped":
      return { startEnabled: true, stopEnabled: false, removeEnabled: true };
    default:
      return { startEnabled: false, stopEnabled: false, removeEnabled: false };
  }
}
