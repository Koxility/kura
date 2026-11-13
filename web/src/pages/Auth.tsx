import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { FormEvent } from "react";
import { api } from "../api";
import ui from "../ui.module.css";
import styles from "./Auth.module.css";

/** First-run setup or login; both take a name and a password. */
export function Auth({ setup }: { setup: boolean }) {
  const client = useQueryClient();
  const submit = useMutation({
    mutationFn: (form: { name: string; password: string }) =>
      api(setup ? "/setup" : "/login", "POST", form),
    onSuccess: () => client.invalidateQueries({ queryKey: ["status"] }),
  });

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    submit.mutate({ name: String(form.get("name")), password: String(form.get("password")) });
  }

  return (
    <main className={styles.page}>
      <form className={styles.form} onSubmit={onSubmit}>
        <h1>{setup ? "Set up Kura" : "Kura"}</h1>
        <p className={ui.lead}>
          {setup
            ? "Create the admin account. It manages settings and other users."
            : "Log in to continue."}
        </p>
        <label className={ui.field}>
          Name
          <input className={ui.input} name="name" autoComplete="username" required autoFocus />
        </label>
        <label className={ui.field}>
          Password
          <input
            className={ui.input}
            name="password"
            type="password"
            autoComplete={setup ? "new-password" : "current-password"}
            minLength={setup ? 8 : undefined}
            required
          />
        </label>
        {submit.isError && (
          <p className={ui.error} role="alert">
            {submit.error.message}
          </p>
        )}
        <div className={ui.actions}>
          <button className={ui.primary} disabled={submit.isPending}>
            {setup ? "Create account" : "Log in"}
          </button>
        </div>
      </form>
    </main>
  );
}
