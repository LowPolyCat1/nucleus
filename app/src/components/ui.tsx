import type { JSX } from "@solidjs/web";
import { Show } from "solid-js";

type ButtonProps = JSX.ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary" | "danger" | "ghost";
  size?: "sm" | "md";
};

export function Button(props: ButtonProps) {
  return (
    <button
      type="button"
      {...props}
      class={[
        "inline-flex items-center justify-center gap-1.5 rounded-md font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50",
        props.size === "sm" ? "px-2 py-1 text-xs" : "px-3 py-1.5 text-sm",
        {
          "bg-indigo-600 text-white hover:bg-indigo-500": (props.variant ?? "primary") === "primary",
          "border border-zinc-700 bg-zinc-800 text-zinc-200 hover:bg-zinc-700": props.variant === "secondary",
          "bg-red-600/90 text-white hover:bg-red-500": props.variant === "danger",
          "text-zinc-400 hover:bg-zinc-800 hover:text-zinc-100": props.variant === "ghost",
        },
        props.class,
      ]}
    />
  );
}

export function Badge(props: { tone?: "zinc" | "indigo" | "emerald" | "amber" | "red" | "sky"; children: JSX.Element; testid?: string }) {
  return (
    <span
      data-testid={props.testid}
      class={[
        "inline-flex items-center rounded px-1.5 py-0.5 font-mono text-[11px] leading-none",
        {
          "bg-zinc-800 text-zinc-300": (props.tone ?? "zinc") === "zinc",
          "bg-indigo-500/15 text-indigo-300": props.tone === "indigo",
          "bg-emerald-500/15 text-emerald-300": props.tone === "emerald",
          "bg-amber-500/15 text-amber-300": props.tone === "amber",
          "bg-red-500/15 text-red-300": props.tone === "red",
          "bg-sky-500/15 text-sky-300": props.tone === "sky",
        },
      ]}
    >
      {props.children}
    </span>
  );
}

export function Spinner(props: { label?: string }) {
  return (
    <span class="inline-flex items-center gap-2 text-sm text-zinc-400" role="status">
      <span class="size-3 animate-spin rounded-full border-2 border-zinc-600 border-t-indigo-400" />
      <Show when={props.label}>{(l) => <span>{l()}</span>}</Show>
    </span>
  );
}

export function Empty(props: { title: string; children?: JSX.Element }) {
  return (
    <div class="flex flex-col items-center justify-center gap-2 p-10 text-center text-sm text-zinc-500">
      <p class="font-medium text-zinc-300">{props.title}</p>
      {props.children}
    </div>
  );
}

export function ErrorBox(props: { message: string; testid?: string }) {
  return (
    <div data-testid={props.testid ?? "error-box"} role="alert" class="rounded-md border border-red-900/60 bg-red-950/40 p-3 text-sm whitespace-pre-wrap text-red-200">
      {props.message}
    </div>
  );
}

export function Modal(props: { title: string; onClose: () => void; children: JSX.Element; testid?: string }) {
  return (
    <div class="fixed inset-0 z-40 flex items-center justify-center bg-black/60 p-4" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
      <div role="dialog" aria-label={props.title} data-testid={props.testid} class="max-h-[85vh] w-full max-w-xl overflow-y-auto rounded-lg border border-zinc-800 bg-zinc-900 p-5 shadow-2xl">
        <div class="mb-4 flex items-center justify-between">
          <h2 class="text-base font-semibold text-zinc-100">{props.title}</h2>
          <button type="button" aria-label="Close dialog" class="text-zinc-500 hover:text-zinc-200" onClick={() => props.onClose()}>
            ✕
          </button>
        </div>
        {props.children}
      </div>
    </div>
  );
}

/** Input styling without a width, for inline controls. */
export const inputBase =
  "rounded-md border border-zinc-700 bg-zinc-950 px-2.5 py-1.5 text-sm text-zinc-100 placeholder:text-zinc-600 focus:border-indigo-500 focus:outline-none";

export const inputClass = `${inputBase} w-full`;

export function errorText(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}
