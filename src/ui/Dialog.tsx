import { ReactNode, useEffect, useRef } from "react";
import { portraitURL } from "../office/art";

export function Portrait({ who, size = 28, tone }: { who: { name: string; role: string }; size?: number; tone?: string }) {
  return (
    <span className={`portrait${tone ? ` t-${tone}` : ""}`} style={{ width: size, height: size }}>
      <img src={portraitURL(who)} alt="" width={size} height={size} />
    </span>
  );
}

/** The card you get when you talk to someone: a game-style dialog anchored to the bottom. */
export function Dialog({
  label,
  head,
  title,
  sub,
  onClose,
  children,
  footer,
}: {
  label: string;
  head: ReactNode;
  title: ReactNode;
  sub?: ReactNode;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
}) {
  const ref = useRef<HTMLElement>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // Escape closes an open skill list first, not the whole card.
        if (document.querySelector(".slash-menu, .more-menu")) return;
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  return (
    <section className="dialog" role="dialog" aria-label={label} ref={ref}>
      <header className="dialog-head">
        {head}
        <div className="dialog-title">
          <h2>{title}</h2>
          {sub && <p>{sub}</p>}
        </div>
        <button className="close" onClick={onClose} aria-label="Close">
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <path d="M2 2l8 8M10 2l-8 8" />
          </svg>
        </button>
      </header>
      <div className="dialog-body">{children}</div>
      {footer && <footer className="dialog-foot">{footer}</footer>}
    </section>
  );
}
