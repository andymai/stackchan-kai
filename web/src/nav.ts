import { createSignal, onCleanup } from "solid-js";

export type SectionId =
  | "status"
  | "behavior"
  | "motion"
  | "voice"
  | "vision"
  | "diagnostics"
  | "system";

export type Section = {
  id: SectionId;
  label: string;
  hotkey: string;
  glyph: string;
};

export const SECTIONS: readonly Section[] = [
  { id: "status", label: "Status", hotkey: "s", glyph: "◇" },
  { id: "behavior", label: "Behavior", hotkey: "b", glyph: "◉" },
  { id: "motion", label: "Motion", hotkey: "m", glyph: "↻" },
  { id: "voice", label: "Voice", hotkey: "v", glyph: "≋" },
  { id: "vision", label: "Vision", hotkey: "y", glyph: "▣" },
  { id: "diagnostics", label: "Diagnostics", hotkey: "d", glyph: "✦" },
  { id: "system", label: "System", hotkey: "c", glyph: "⚙" },
] as const;

const hashId = (): SectionId => {
  const h = location.hash.replace(/^#/, "");
  return SECTIONS.some((s) => s.id === h) ? (h as SectionId) : "status";
};

export const [section, setSection] = createSignal<SectionId>(hashId());

export function goto(id: SectionId): void {
  setSection(id);
  if (location.hash !== `#${id}`) location.hash = id;
}

export function bindHashRouter(): () => void {
  const onHash = () => setSection(hashId());
  window.addEventListener("hashchange", onHash);
  return () => window.removeEventListener("hashchange", onHash);
}

export function useHashRouter(): void {
  onCleanup(bindHashRouter());
}
