import {
  createContext,
  forwardRef,
  useCallback,
  useContext,
  useEffect,
  useId,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
  type TextareaHTMLAttributes,
} from "react";
import { X } from "lucide-react";
import { Link } from "react-router";

export function cx(...classes: (string | false | null | undefined)[]): string {
  return classes.filter(Boolean).join(" ");
}

type Variant = "primary" | "secondary" | "ghost" | "danger";

const variants: Record<Variant, string> = {
  primary: "bg-signal text-signal-ink hover:brightness-95 active:brightness-90",
  secondary: "bg-surface-2 text-ink hover:bg-surface-3",
  ghost: "bg-transparent text-ink hover:bg-surface-2",
  danger: "bg-danger text-white hover:brightness-110 dark:text-black",
};

export function buttonClass(variant: Variant = "secondary", size: "sm" | "md" = "md", className?: string) {
  return cx(
    "inline-flex items-center justify-center gap-2 font-medium transition-[filter,background-color,transform] duration-150 active:translate-y-px disabled:cursor-not-allowed disabled:opacity-50",
    size === "md" ? "h-11 px-5 text-base" : "h-9 px-3 text-sm",
    variants[variant],
    className,
  );
}

/** A router link that looks like a button (never a button inside a link). */
export function LinkButton({ to, variant, size, className, children }: { to: string; variant?: Variant; size?: "sm" | "md"; className?: string; children: ReactNode }) {
  return (
    <Link to={to} className={buttonClass(variant, size, className)}>
      {children}
    </Link>
  );
}

export const Button = forwardRef<HTMLButtonElement, ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; size?: "sm" | "md" }>(
  function Button({ variant = "secondary", size = "md", className, type = "button", ...rest }, ref) {
    return (
      <button
        ref={ref}
        type={type}
        className={buttonClass(variant, size, className)}
        {...rest}
      />
    );
  },
);

const fieldBase =
  "w-full bg-surface text-ink border border-surface-3 px-3 h-11 text-base placeholder:text-ink-soft focus:outline-2 focus:outline-[var(--focus)] focus:-outline-offset-1 aria-[invalid=true]:border-danger";

export const Input = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(function Input({ className, ...rest }, ref) {
  return <input ref={ref} className={cx(fieldBase, className)} {...rest} />;
});

export const Select = forwardRef<HTMLSelectElement, SelectHTMLAttributes<HTMLSelectElement>>(function Select({ className, ...rest }, ref) {
  return <select ref={ref} className={cx(fieldBase, "pr-8", className)} {...rest} />;
});

export const Textarea = forwardRef<HTMLTextAreaElement, TextareaHTMLAttributes<HTMLTextAreaElement>>(function Textarea({ className, ...rest }, ref) {
  return <textarea ref={ref} className={cx(fieldBase, "h-auto min-h-24 py-2", className)} {...rest} />;
});

export function Field({ label, hint, error, children, htmlFor }: { label: string; hint?: ReactNode; error?: string; children: ReactNode; htmlFor?: string }) {
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={htmlFor} className="text-base font-medium">
        {label}
      </label>
      {children}
      {error ? (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      ) : hint ? (
        <p className="text-sm text-ink-soft">{hint}</p>
      ) : null}
    </div>
  );
}

export function Checkbox({ label, hint, ...rest }: InputHTMLAttributes<HTMLInputElement> & { label: string; hint?: string }) {
  const id = useId();
  return (
    <div className="flex items-start gap-3">
      <input id={id} type="checkbox" className="mt-1 size-4 accent-[var(--ink)]" {...rest} />
      <label htmlFor={id} className="text-base">
        {label}
        {hint && <span className="block text-sm text-ink-soft">{hint}</span>}
      </label>
    </div>
  );
}

export function PageHeader({ title, lead, actions }: { title: string; lead?: ReactNode; actions?: ReactNode }) {
  return (
    <header className="rise mb-8 flex flex-col gap-4 md:mb-12 md:flex-row md:items-end md:justify-between">
      <div className="max-w-2xl">
        <h1 className="text-title font-semibold">{title}</h1>
        {lead && <p className="mt-3 text-lg text-ink-soft">{lead}</p>}
      </div>
      {actions && <div className="flex flex-wrap gap-2">{actions}</div>}
    </header>
  );
}

export function Panel({ children, className, title, actions }: { children: ReactNode; className?: string; title?: string; actions?: ReactNode }) {
  return (
    <section className={cx("bg-surface p-5 md:p-7", className)}>
      {(title || actions) && (
        <div className="mb-5 flex items-center justify-between gap-4">
          {title && <h2 className="text-xl font-semibold tracking-[-0.01em]">{title}</h2>}
          {actions}
        </div>
      )}
      {children}
    </section>
  );
}

export function Empty({ title, children, action }: { title: string; children?: ReactNode; action?: ReactNode }) {
  return (
    <div className="bg-surface-2 px-6 py-12 md:px-10 md:py-16">
      <h2 className="text-2xl font-semibold tracking-[-0.02em]">{title}</h2>
      {children && <div className="mt-3 max-w-xl text-lg">{children}</div>}
      {action && <div className="mt-6">{action}</div>}
    </div>
  );
}

export function Loading({ label = "Loading" }: { label?: string }) {
  return (
    <div role="status" aria-live="polite" className="flex items-center gap-3 py-10 text-ink-soft">
      <span className="inline-block size-3 bg-signal" aria-hidden />
      {label}…
    </div>
  );
}

export function ErrorNote({ error, retry }: { error: unknown; retry?: () => void }) {
  const message = error instanceof Error ? error.message : "Something went wrong.";
  return (
    <div role="alert" className="flex flex-wrap items-center justify-between gap-4 bg-danger-soft px-5 py-4 text-ink">
      <span>{message}</span>
      {retry && (
        <Button size="sm" onClick={retry}>
          Try again
        </Button>
      )}
    </div>
  );
}

export function Tag({ tone = "neutral", children }: { tone?: "neutral" | "signal" | "ok" | "danger"; children: ReactNode }) {
  const tones = { neutral: "bg-surface-2 text-ink", signal: "bg-signal text-signal-ink", ok: "bg-ok-soft text-ok", danger: "bg-danger-soft text-danger" };
  return <span className={cx("inline-flex items-center px-2 py-0.5 text-sm font-medium", tones[tone])}>{children}</span>;
}

export function Person({ name, size = "md" }: { name: string; size?: "sm" | "md" }) {
  const initial = name.trim().charAt(0).toUpperCase() || "?";
  return (
    <span className="inline-flex items-center gap-2">
      <span aria-hidden className={cx("inline-flex shrink-0 items-center justify-center bg-surface-3 font-semibold", size === "sm" ? "size-6 text-xs" : "size-8 text-sm")}>
        {initial}
      </span>
      <span>{name}</span>
    </span>
  );
}

// ---------- dialogs ----------

export function Dialog({ open, onClose, title, children, wide }: { open: boolean; onClose: () => void; title: string; children: ReactNode; wide?: boolean }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (open && !el.open) el.showModal();
    if (!open && el.open) el.close();
  }, [open]);
  return (
    <dialog
      ref={ref}
      onClose={onClose}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      className={cx("m-auto max-h-[90vh] w-[calc(100%-32px)] overflow-y-auto bg-surface p-0 text-ink shadow-2xl", wide ? "max-w-3xl" : "max-w-lg") /* taste-skill-ignore: modal dialog */}
    >
      {open && (
        <div className="p-6 md:p-8">
          <div className="mb-6 flex items-start justify-between gap-4">
            <h2 className="text-2xl font-semibold tracking-[-0.02em]">{title}</h2>
            <button type="button" onClick={onClose} className="-mr-2 -mt-1 p-2 hover:bg-surface-2" aria-label="Close">
              <X className="size-5" />
            </button>
          </div>
          {children}
        </div>
      )}
    </dialog>
  );
}

export function ConfirmDialog({
  open,
  title,
  children,
  confirmLabel,
  onConfirm,
  onClose,
  busy,
}: {
  open: boolean;
  title: string;
  children: ReactNode;
  confirmLabel: string;
  onConfirm: () => void;
  onClose: () => void;
  busy?: boolean;
}) {
  return (
    <Dialog open={open} onClose={onClose} title={title}>
      <div className="text-lg">{children}</div>
      <div className="mt-8 flex justify-end gap-2">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="danger" onClick={onConfirm} disabled={busy}>
          {confirmLabel}
        </Button>
      </div>
    </Dialog>
  );
}

// ---------- toasts ----------

type Toast = { id: number; tone: "ok" | "danger" | "neutral"; text: string };
const ToastContext = createContext<(text: string, tone?: Toast["tone"]) => void>(() => {});

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const push = useCallback((text: string, tone: Toast["tone"] = "ok") => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t, { id, tone, text }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), tone === "danger" ? 8000 : 4000);
  }, []);
  return (
    <ToastContext.Provider value={push}>
      {children}
      <div aria-live="polite" className="pointer-events-none fixed inset-x-4 bottom-4 z-50 flex flex-col items-end gap-2 md:inset-x-auto md:right-6 md:bottom-6">
        {toasts.map((t) => (
          <div
            key={t.id}
            role={t.tone === "danger" ? "alert" : "status"}
            className={cx(
              "rise pointer-events-auto max-w-sm px-5 py-3 text-base shadow-xl", // taste-skill-ignore: floating toast
              t.tone === "ok" && "bg-ink text-ground",
              t.tone === "danger" && "bg-danger text-white dark:text-black",
              t.tone === "neutral" && "bg-surface-3 text-ink",
            )}
          >
            {t.text}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast() {
  return useContext(ToastContext);
}
