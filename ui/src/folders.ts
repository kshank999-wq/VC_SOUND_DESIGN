// Track folder tree for the timeline (unit-tested in tests/folders.test.ts).
// Folders are "/"-separated paths on each track; the tree keeps the order in
// which folders and tracks first appear.

export interface FolderTrack {
  id: string;
  folder: string;
}

export type TreeItem<T extends FolderTrack> =
  | { kind: "folder"; path: string; name: string; depth: number; collapsed: boolean; trackIds: string[] }
  | { kind: "track"; track: T; depth: number };

interface Node<T> {
  path: string;
  name: string;
  items: ({ folder: Node<T> } | { track: T })[];
  index: Map<string, Node<T>>;
}

export function splitPath(folder: string): string[] {
  return folder.split("/").map((s) => s.trim()).filter(Boolean);
}

export function parentOf(path: string): string {
  const p = splitPath(path);
  return p.slice(0, -1).join("/");
}

export function folderTree<T extends FolderTrack>(tracks: T[], collapsed: Set<string>): TreeItem<T>[] {
  const root: Node<T> = { path: "", name: "", items: [], index: new Map() };
  for (const t of tracks) {
    let node = root;
    for (const seg of splitPath(t.folder)) {
      let next = node.index.get(seg);
      if (!next) {
        next = { path: node.path ? `${node.path}/${seg}` : seg, name: seg, items: [], index: new Map() };
        node.index.set(seg, next);
        node.items.push({ folder: next });
      }
      node = next;
    }
    node.items.push({ track: t });
  }
  const ids = (n: Node<T>): string[] => n.items.flatMap((i) => ("track" in i ? [i.track.id] : ids(i.folder)));
  const out: TreeItem<T>[] = [];
  const walk = (n: Node<T>, depth: number) => {
    for (const i of n.items) {
      if ("track" in i) {
        out.push({ kind: "track", track: i.track, depth });
      } else {
        const f = i.folder;
        const isCollapsed = collapsed.has(f.path);
        out.push({ kind: "folder", path: f.path, name: f.name, depth, collapsed: isCollapsed, trackIds: ids(f) });
        if (!isCollapsed) walk(f, depth + 1);
      }
    }
  };
  walk(root, 0);
  return out;
}

/** Every folder path that exists (including parents), sorted. */
export function allFolders(tracks: FolderTrack[]): string[] {
  const set = new Set<string>();
  for (const t of tracks) {
    const p = splitPath(t.folder);
    for (let i = 1; i <= p.length; i++) set.add(p.slice(0, i).join("/"));
  }
  return [...set].sort();
}
