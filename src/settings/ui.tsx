import { useEffect, useRef, useState, type ReactNode } from 'react';
import { iconInner } from '../shared/icons';

export function Icon({ name, size = 16 }: { name: string; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      dangerouslySetInnerHTML={{ __html: iconInner(name) }}
    />
  );
}

export function Switch({ checked, onChange, label }: { checked: boolean; onChange(next: boolean): void; label: string }) {
  return <button type="button" role="switch" className="switch" aria-checked={checked} aria-label={label} onClick={() => onChange(!checked)} />;
}

export function Setting({ title, help, children }: { title: string; help?: ReactNode; children: ReactNode }) {
  return (
    <div className="setting">
      <div>
        <b>{title}</b>
        {help && <small>{help}</small>}
      </div>
      <div className="control">{children}</div>
    </div>
  );
}

/**
 * A native modal dialog. Opens with the element, closes on Escape at once (a
 * key press never waits for an animation; the exit transition is cosmetic and
 * the dialog is already inert).
 */
export function Dialog({ open, onClose, wide, children }: { open: boolean; onClose(): void; wide?: boolean; children: ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  // Content stays mounted through the 150 ms exit so the dialog does not
  // empty out while it fades, then goes, taking its effects with it.
  const [present, setPresent] = useState(open);
  const last = useRef<ReactNode>(children);
  if (open) last.current = children;
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open) {
      setPresent(true);
      if (!d.open) d.showModal();
      return;
    }
    if (d.open) d.close();
    const t = window.setTimeout(() => setPresent(false), 170);
    return () => window.clearTimeout(t);
  }, [open]);
  return (
    <dialog
      ref={ref}
      className={wide ? 'wide' : undefined}
      onClose={onClose}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      onClick={(e) => {
        // A click on the backdrop lands on the dialog element itself.
        if (e.target === ref.current) onClose();
      }}
    >
      {(open || present) && <div className="dialog-scroll">{open ? children : last.current}</div>}
    </dialog>
  );
}
