/**
 * TypeScript bindings for `tauri-plugin-proxmark3`.
 *
 * ```ts
 * import { connect, execute } from 'tauri-plugin-proxmark3-api';
 *
 * await connect();                  // offline session
 * const { output } = await execute('hw version');
 * ```
 */

import { invoke } from '@tauri-apps/api/core';

/** State of the Proxmark3 client session. */
export interface SessionInfo {
  /** A session is open (offline sessions included). */
  open: boolean;
  /** The session runs without a device. */
  offline: boolean;
  /** Transport of the session, `''` when offline. */
  port: string;
  /** Proxmark3 client version compiled into the library. */
  version: string;
}

/** Result of one command execution. */
export interface CommandOutput {
  /** proxmark3 client status code (0 == success). */
  status: number;
  /** Everything the client printed, ANSI sequences removed. */
  output: string;
  /** Wall clock duration in milliseconds. */
  durationMs: number;
}

/**
 * Opens a Proxmark3 session.
 *
 * @param port `undefined` for an offline session or auto-detection, a serial
 *   port (`/dev/ttyACM0`, `COM3`), or a network transport (`tcp:host:port`,
 *   `udp:host:port`).
 */
export async function connect(port?: string | null): Promise<SessionInfo> {
  return invoke<SessionInfo>('plugin:proxmark3|connect', { port: port ?? null });
}

/** Closes the current session. */
export async function disconnect(): Promise<SessionInfo> {
  return invoke<SessionInfo>('plugin:proxmark3|disconnect');
}

/**
 * Executes one client command, using the same syntax as the `pm3` client.
 * Opens an offline session automatically when no session is open yet.
 */
export async function execute(command: string): Promise<CommandOutput> {
  return invoke<CommandOutput>('plugin:proxmark3|execute', { command });
}

/** Current session state. */
export async function status(): Promise<SessionInfo> {
  return invoke<SessionInfo>('plugin:proxmark3|status');
}

/** Client version string compiled into the library. */
export async function version(): Promise<string> {
  return invoke<string>('plugin:proxmark3|version');
}
