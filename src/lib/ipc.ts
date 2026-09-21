// Typed wrapper over the ShellCommands surface.
// Contract: specs/001-app-shell/contracts/shell-commands.md
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type RegionId = 'navigation' | 'output' | 'document_area';
export type ConnectionState = 'unknown' | 'connecting' | 'connected' | 'disconnected';
export type LocationType = 'REMOTE' | 'LOCAL';

export interface RegionState {
  visible: boolean;
  extent: number;
}

export interface WindowGeometry {
  x: number;
  y: number;
  width: number;
  height: number;
  maximized: boolean;
}

export interface Layout {
  navigation: RegionState;
  output: RegionState;
  document_area: RegionState;
}

export interface OpenDocumentReference {
  id: string;
  display_name: string;
  order: number;
}

export interface WorkspaceReference {
  name: string;
  location_type: LocationType;
}

/** Deliberately carries no schema_version: storage format is not the interface's concern. */
export interface SessionSnapshot {
  workspace: WorkspaceReference | null;
  window: WindowGeometry;
  layout: Layout;
  documents: OpenDocumentReference[];
  focused_document_id: string | null;
}

export const shellReady = (): Promise<void> => invoke('shell_ready');
export const sessionGet = (): Promise<SessionSnapshot> => invoke('session_get');

export const layoutSetRegion = (region: RegionId, visible: boolean, extent: number): Promise<void> =>
  invoke('layout_set_region', { region, visible, extent });

export const documentsOpen = (displayName: string): Promise<string> =>
  invoke('documents_open', { displayName });
export const documentsClose = (id: string): Promise<void> => invoke('documents_close', { id });
export const documentsReorder = (id: string, toOrder: number): Promise<void> =>
  invoke('documents_reorder', { id, toOrder });
export const documentsFocus = (id: string): Promise<void> => invoke('documents_focus', { id });

export const onConnectionChanged = (
  handler: (state: ConnectionState) => void,
): Promise<UnlistenFn> =>
  listen<ConnectionState>('connection:changed', (e) => handler(e.payload));
