import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { api } from "../api";
import ui from "../ui.module.css";
import styles from "./Auth.module.css";

/** First-run setup or login; both take a name and a password. With a mail server set up, the
 *  login also offers a reset link for a forgotten password. */
export function Auth({ setup, passwordReset }: { setup: boolean; passwordReset: boolean }) {
  const [forgot, setForgot] = useState(false);
  if (forgot) return <Forgot onBack={() => setForgot(false)} />;
  return <Login setup={setup} passwordReset={passwordReset} onForgot={() => setForgot(true)} />;
}

function Login({
  setup,
  passwordReset,
  onForgot,
}: {
  setup: boolean;
  passwordReset: boolean;
  onForgot: () => void;
}) {
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
        {!setup &&
          (passwordReset ? (
            <p className={styles.aside}>
              <button type="button" className={styles.link} onClick={onForgot}>
                Forgot password?
              </button>
            </p>
          ) : (
            <p className={styles.aside}>Forgot your password? An admin can set a new one.</p>
          ))}
      </form>
    </main>
  );
}

/** Asks for a reset link. The answer reads the same whether or not the account exists. */
function Forgot({ onBack }: { onBack: () => void }) {
  const ask = useMutation({
    mutationFn: (name: string) => api<{ message: string }>("/password-reset", "POST", { name }),
  });

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    ask.mutate(String(new FormData(event.currentTarget).get("name")));
  }

  return (
    <main className={styles.page}>
      <form className={styles.form} onSubmit={onSubmit}>
        <h1>Kura</h1>
        <p className={ui.lead}>
          Enter your name or email address. If your account has an email address, a link to choose
          a new password is sent there.
        </p>
        {ask.isSuccess ? (
          <p className={styles.sent} role="status">
            {ask.data.message}
          </p>
        ) : (
          <>
            <label className={ui.field}>
              Name or email
              <input className={ui.input} name="name" autoComplete="username" required autoFocus />
            </label>
            {ask.isError && (
              <p className={ui.error} role="alert">
                {ask.error.message}
              </p>
            )}
            <div className={ui.actions}>
              <button className={ui.primary} disabled={ask.isPending}>
                Send link
              </button>
            </div>
          </>
        )}
        <p className={styles.aside}>
          <button type="button" className={styles.link} onClick={onBack}>
            Back to login
          </button>
        </p>
      </form>
    </main>
  );
}
