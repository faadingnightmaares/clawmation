// Keyboard event → the shortcut string the backend stores.
//
// Rust upper-cases the stored string and feeds it to `Shortcut::from_str`
// (global-hotkey's parser). A name outside that parser's vocabulary registers
// nothing and reports nothing, so capture only ever emits names it accepts and
// refuses everything else; a rejected keystroke is better than a shortcut that
// silently never fires.

/** Physical `KeyboardEvent.code`s that only ever act as modifiers. */
const MODIFIER_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "ShiftLeft",
  "ShiftRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
]);

/** `event.code` → parser name, for the keys that aren't a simple pattern. */
const NAMED_KEYS: Record<string, string> = {
  Escape: "esc",
  Space: "space",
  Enter: "enter",
  NumpadEnter: "numenter",
  Tab: "tab",
  Backspace: "backspace",
  Delete: "delete",
  Insert: "insert",
  Home: "home",
  End: "end",
  PageUp: "pageup",
  PageDown: "pagedown",
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
  Backquote: "backquote",
  Backslash: "backslash",
  BracketLeft: "bracketleft",
  BracketRight: "bracketright",
  Comma: "comma",
  Period: "period",
  Minus: "minus",
  Equal: "equal",
  Quote: "quote",
  Semicolon: "semicolon",
  Slash: "slash",
  CapsLock: "capslock",
  NumLock: "numlock",
  ScrollLock: "scrolllock",
  PrintScreen: "printscreen",
  Pause: "pause",
  NumpadAdd: "numadd",
  NumpadSubtract: "numsubtract",
  NumpadMultiply: "nummultiply",
  NumpadDivide: "numdivide",
  NumpadDecimal: "numdecimal",
};

/** The non-modifier half of a shortcut, or null if the parser can't take it. */
function baseKey(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3).toLowerCase();
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^Numpad[0-9]$/.test(code)) return `num${code.slice(6)}`;
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code.toLowerCase();
  return NAMED_KEYS[code] ?? null;
}

export interface KeyLike {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

export function isModifierOnly(code: string): boolean {
  return MODIFIER_CODES.has(code);
}

/**
 * The shortcut a keystroke means, e.g. `ctrl+shift+r` or `f6`, or null while
 * only modifiers are down, and for keys the backend parser doesn't know.
 * Modifier order is fixed so the same chord always stores the same string.
 */
export function accelFromEvent(e: KeyLike): string | null {
  const key = baseKey(e.code);
  if (key === null) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("ctrl");
  if (e.altKey) parts.push("alt");
  if (e.shiftKey) parts.push("shift");
  if (e.metaKey) parts.push("super");
  parts.push(key);
  return parts.join("+");
}

const CAP_LABELS: Record<string, string> = {
  ctrl: "Ctrl",
  alt: "Alt",
  shift: "Shift",
  super: "Win",
  esc: "Esc",
  space: "Space",
  enter: "Enter",
  numenter: "Num Enter",
  tab: "Tab",
  backspace: "Backspace",
  delete: "Del",
  insert: "Ins",
  home: "Home",
  end: "End",
  pageup: "Page Up",
  pagedown: "Page Dn",
  up: "↑",
  down: "↓",
  left: "←",
  right: "→",
  backquote: "`",
  backslash: "\\",
  bracketleft: "[",
  bracketright: "]",
  comma: ",",
  period: ".",
  minus: "−",
  equal: "=",
  quote: "'",
  semicolon: ";",
  slash: "/",
  capslock: "Caps",
  numlock: "Num Lock",
  scrolllock: "Scroll Lock",
  printscreen: "Print Scrn",
  pause: "Pause",
  numadd: "Num +",
  numsubtract: "Num −",
  nummultiply: "Num ×",
  numdivide: "Num ÷",
  numdecimal: "Num .",
};

/**
 * A stored shortcut split into the caps to render, e.g. `ctrl+shift+r` →
 * `["Ctrl", "Shift", "R"]`. Unknown tokens (a hand-edited config) are shown
 * as-is rather than dropped, so the user can see what's actually saved.
 */
export function accelCaps(accel: string): string[] {
  if (!accel.trim()) return [];
  return accel
    .split("+")
    .map((raw) => raw.trim().toLowerCase())
    .filter(Boolean)
    .map((token) => {
      if (CAP_LABELS[token]) return CAP_LABELS[token];
      if (/^f\d{1,2}$/.test(token)) return token.toUpperCase();
      if (/^num\d$/.test(token)) return `Num ${token.slice(3)}`;
      return token.length === 1 ? token.toUpperCase() : token;
    });
}
