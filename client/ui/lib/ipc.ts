// Typed wrapper over the ShellCommands surface.
// Contract: specs/001-app-shell/contracts/shell-commands.md
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type RegionId = 'output' | 'document_area';
/// Mirrors `domain::connection::ConnectionState`. The first four serialise as plain strings;
/// `Retrying` carries data, so serde renders it as an object. Widening this type is what
/// stops the status bar indexing a record with an object key and rendering nothing.
export type ConnectionState =
  | 'unknown'
  | 'connecting'
  | 'connected'
  | 'disconnected'
  | { retrying: { attempt: number; next_in_secs: number } }
  | { deploying: { sent: number; total: number } };
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
  output: RegionState;
  document_area: RegionState;
}

export interface OpenDocumentReference {
  /// Which file the tab is of. Separate from `display_name`, which is what a person reads: a
  /// tab that remembered only its name could be restored as a label with nothing behind it.
  path: string;
  id: string;
  display_name: string;
  order: number;
}

export interface WorkspaceReference {
  /// The identity the tree and the file commands key on. Empty for a session written before
  /// schema version 4, where it was never recorded.
  id: string;
  name: string;
  location_type: LocationType;
}

/** Deliberately carries no schema_version: storage format is not the interface's concern. */
export interface SessionSnapshot {
  /// Whether saves happen without being asked (FR-007b). Off for a profile that never set it:
  /// autosave writes the developer's file on its own, and never having answered is not consent.
  autosave: boolean;
  workspace: WorkspaceReference | null;
  window: WindowGeometry;
  layout: Layout;
  documents: OpenDocumentReference[];
  focused_document_id: string | null;
  tool_window: ToolWindowState;
}

export interface RailDestination {
  id: string;
  label: string;
  icon: string;
  available: boolean;
  order: number;
}

export interface ToolWindowState {
  active_destination_id: string | null;
  collapsed: boolean;
  width: number;
}

export const shellReady = (): Promise<void> => invoke('shell_ready');
export const sessionGet = (): Promise<SessionSnapshot> => invoke('session_get');

export const sessionSetAutosave = (on: boolean): Promise<void> =>
  invoke('session_set_autosave', { on });

export const layoutSetRegion = (
  region: RegionId,
  visible: boolean,
  extent: number,
): Promise<void> => invoke('layout_set_region', { region, visible, extent });

export const documentsOpen = (displayName: string, path: string): Promise<string> =>
  invoke('documents_open', { displayName, path });
export const documentsClose = (id: string): Promise<void> => invoke('documents_close', { id });
export const documentsReorder = (id: string, toOrder: number): Promise<void> =>
  invoke('documents_reorder', { id, toOrder });
export const documentsFocus = (id: string): Promise<void> => invoke('documents_focus', { id });

export const onConnectionChanged = (
  handler: (state: ConnectionState) => void,
): Promise<UnlistenFn> => listen<ConnectionState>('connection:changed', (e) => handler(e.payload));

export const onWorkspaceChanged = (
  handler: (workspace: WorkspaceReference | null) => void,
): Promise<UnlistenFn> =>
  listen<WorkspaceReference | null>('workspace:changed', (e) => handler(e.payload));

export const railSelect = (destinationId: string): Promise<ToolWindowState> =>
  invoke('rail_select', { destinationId });
export const toolWindowResize = (width: number): Promise<void> =>
  invoke('tool_window_resize', { width });
export const railDestinations = (): Promise<RailDestination[]> => invoke('rail_destinations');
