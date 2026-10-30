import { invoke } from '@tauri-apps/api/core';

// All UI requests use the same Rust command. Keep the transport at this boundary.
export function api<T = boolean>(op: string, args: unknown = {}): Promise<T> {
  return invoke('request', { op, args });
}
