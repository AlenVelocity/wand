import { useEffect, useRef, useState } from 'react';
import { ICON_NAMES, suggestIcon } from '../shared/icons';
import type { Action } from '../shared/types';
import { backend } from './api';
import { isMac } from './platform';
import { Icon } from './ui';

const KINDS: { type: Action['type']; label: string }[] = [
  { type: 'key_combo', label: 'Keys' },
  { type: 'media_key', label: 'Media' },
  { type: 'launch', label: 'Open' },
  { type: 'shell', label: 'Command' },
];

function blank(type: Action['type']): Action {
  switch (type) {
    case 'key_combo':
      return { type, keys: '' };
    case 'media_key':
      return { type, key: 'play_pause', repeat: 1 };
    case 'launch':
      return { type, path: '', args: '' };
    case 'shell':
      return { type, command: '' };
    default:
      return { type: 'none' };
  }
}

const CODE_NAMES: Record<string, string> = {
  ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right', Escape: 'Esc', Minus: '-', Equal: '=', Comma: ',', Period: '.',
  Slash: '/', Semicolon: ';', Quote: "'", Backquote: '`', BracketLeft: '[', BracketRight: ']', Backslash: '\\',
};

/** A pressed key as the combo text the backend parses, or null for a bare modifier. */
function keyName(e: KeyboardEvent): string | null {
  // `code` names the physical key; it is empty for some injected or remapped
  // keys, in which case `key` is the best there is.
  const c = e.code || (e.key.length === 1 ? e.key.toUpperCase() : e.key);
  if (/^(Control|Shift|Alt|Meta|OS)(Left|Right)?$/.test(c)) return null;
  if (c.startsWith('Key')) return c.slice(3);
  if (c.startsWith('Digit')) return c.slice(5);
  return CODE_NAMES[c] ?? c;
}

export function Keys({ combo }: { combo: string }) {
  const parts = combo.split('+').map((p) => p.trim()).filter(Boolean);
  if (parts.length === 0) return null;
  return (
    <span className="keys">
      {parts.map((p, i) => (
        <kbd key={i}>{p}</kbd>
      ))}
    </span>
  );
}

interface Props {
  action: Action;
  label: string;
  icon: string;
  mediaKeys: [string, string][];
  onChange(next: { action: Action; label: string; icon: string }): void;
}

export function ActionEditor({ action, label, icon, mediaKeys, onChange }: Props) {
  const [listening, setListening] = useState(false);
  const captureRef = useRef<HTMLButtonElement>(null);

  // While listening, the next key press (with whatever modifiers are held)
  // becomes the combination. Escape on its own cancels.
  useEffect(() => {
    if (!listening) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const name = keyName(e);
      if (!name) return;
      const mods = (isMac
        ? [e.ctrlKey && 'Ctrl', e.altKey && 'Option', e.shiftKey && 'Shift', e.metaKey && 'Cmd']
        : [e.ctrlKey && 'Ctrl', e.shiftKey && 'Shift', e.altKey && 'Alt', e.metaKey && 'Win']
      ).filter(Boolean) as string[];
      if (name === 'Esc' && mods.length === 0) {
        setListening(false);
        return;
      }
      onChange({ action: { type: 'key_combo', keys: [...mods, name].join('+') }, label, icon });
      setListening(false);
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [listening, label, icon, onChange]);

  const set = (next: Action) => {
    // Keep a hand-picked icon; otherwise follow the action.
    const auto = icon === '' || icon === suggestIcon(action);
    onChange({ action: next, label, icon: auto ? suggestIcon(next) : icon });
  };

  return (
    <div className="stack">
      <div className="seg" role="group" aria-label="What the gesture does">
        {KINDS.map((k) => (
          <button key={k.type} type="button" aria-pressed={action.type === k.type} onClick={() => action.type !== k.type && set(blank(k.type))}>
            {k.label}
          </button>
        ))}
      </div>

      {action.type === 'key_combo' && (
        <div className="field">
          <span>Key combination</span>
          <div className="row">
            <input
              className="input mono grow"
              value={action.keys}
              placeholder={isMac ? 'Cmd+Shift+S' : 'Ctrl+Shift+S'}
              spellCheck={false}
              onChange={(e) => set({ type: 'key_combo', keys: e.target.value })}
            />
            <button ref={captureRef} type="button" className="btn" onClick={() => setListening((v) => !v)}>
              {listening ? 'Press the keys…' : 'Record keys'}
            </button>
          </div>
          <p className="note">
            {isMac
              ? 'Type it, or record it. Combinations macOS or this window keep for themselves, like Cmd+Tab or Cmd+W, cannot be recorded but can be typed.'
              : 'Type it, or record it. Combinations Windows keeps for itself, like Win+L, cannot be recorded but can be typed.'}
          </p>
        </div>
      )}

      {action.type === 'media_key' && (
        <div className="row">
          <label className="field grow">
            <span>Key</span>
            <select className="select" value={action.key} onChange={(e) => set({ ...action, key: e.target.value })}>
              {mediaKeys.map(([k, name]) => (
                <option key={k} value={k}>
                  {name}
                </option>
              ))}
            </select>
          </label>
          <label className="field" style={{ width: 120 }}>
            <span>Times</span>
            <input
              className="input mono"
              type="number"
              min={1}
              max={20}
              value={action.repeat}
              onChange={(e) => set({ ...action, repeat: Math.max(1, Math.min(20, Number(e.target.value) || 1)) })}
            />
          </label>
        </div>
      )}

      {action.type === 'launch' && (
        <>
          <div className="field">
            <span>Program, file, folder or link</span>
            <div className="row">
              <input className="input mono grow" value={action.path} placeholder="C:\Program Files\…\app.exe" spellCheck={false} onChange={(e) => set({ ...action, path: e.target.value })} />
              <button
                type="button"
                className="btn"
                onClick={async () => {
                  const picked = await (await backend()).pickProgram();
                  if (picked) set({ ...action, path: picked });
                }}
              >
                Browse
              </button>
            </div>
          </div>
          <label className="field">
            <span>Arguments (optional)</span>
            <input className="input mono" value={action.args} spellCheck={false} onChange={(e) => set({ ...action, args: e.target.value })} />
          </label>
        </>
      )}

      {action.type === 'shell' && (
        <label className="field">
          <span>Command line</span>
          <input className="input mono" value={action.command} placeholder="explorer.exe shell:Downloads" spellCheck={false} onChange={(e) => set({ ...action, command: e.target.value })} />
          <p className="note">Runs hidden, with your own permissions, the moment the gesture matches.</p>
        </label>
      )}

      <div className="row" style={{ alignItems: 'flex-start', gap: 20 }}>
        <label className="field grow">
          <span>Label on the overlay</span>
          <input className="input" value={label} placeholder="Shown when the gesture matches" maxLength={28} onChange={(e) => onChange({ action, label: e.target.value, icon })} />
        </label>
        <div className="field">
          <span>Icon</span>
          <div className="icons" style={{ maxWidth: 264 }}>
            {ICON_NAMES.map((name) => (
              <button key={name} type="button" title={name} aria-label={name} aria-pressed={icon === name} onClick={() => onChange({ action, label, icon: name })}>
                <Icon name={name} size={18} />
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
