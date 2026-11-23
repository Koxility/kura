import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { useNavigate } from "react-router";
import { api } from "../api";
import ui from "../ui.module.css";
import styles from "./Auth.module.css";

/** The token of a reset link: in the part after `#`, which browsers never send to a server,
 *  so it does not end up in Kura's or a proxy's logs. */
function tokenFromLink(): string {
  return new URLSearchParams(window.location.hash.slice(1)).get("token") ?? "";
}

/** Where a reset link leads: choose a new password, then log in with it. */
export function ResetPassword() {
  const client = useQueryClient();
  const navigate = useNavigate();
  // Read once, then taken out of the address bar and the history.
  const [token] = useState(() => {
    const token = tokenFromLink();
    if (window.location.hash) {
      window.history.replaceState(null, "", window.location.pathname);
    }
    return token;
  });
  const [password, setPassword] = useState("");
  const [again, setAgain] = useState("");
  const mismatch = again !== "" && again !== password;
  const reset = useMutation({
    mutationFn: () => api("/password-reset/confirm", "POST", { token, password }),
    // Every session ended with the reset; the status shows the login again.
    onSuccess: () => client.resetQueries(),
  });

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    if (!mismatch) reset.mutate();
  }

  const toLogin = (
    <p className={styles.aside}>
      <button type="button" className={styles.link} onClick={() => navigate("/", { replace: true })}>
        {reset.isSuccess ? "Log in" : "Back to login"}
      </button>
    </p>
  );

  return (
    <main className={styles.page}>
      <form className={styles.form} onSubmit={onSubmit}>
        <h1>Kura</h1>
        {!token ? (
          <p className={ui.lead}>This link is incomplete. Ask for a new one on the login page.</p>
        ) : reset.isSuccess ? (
          <p className={styles.sent} role="status">
            Your password is changed. Every device was logged out; log in with the new password.
          </p>
        ) : (
          <>
            <p className={ui.lead}>Choose a new password for your account.</p>
            <label className={ui.field}>
              New password
              <input
                className={ui.input}
                type="password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                autoComplete="new-password"
                minLength={8}
                required
                autoFocus
              />
            </label>
            <label className={ui.field}>
              New password again
              <input
                className={ui.input}
                type="password"
                value={again}
                onChange={(event) => setAgain(event.target.value)}
                autoComplete="new-password"
                aria-invalid={mismatch}
                required
              />
            </label>
            {mismatch && (
              <p className={ui.error} role="alert">
                The new passwords differ.
              </p>
            )}
            {reset.isError && (
              <p className={ui.error} role="alert">
                {reset.error.message}
              </p>
            )}
            <div className={ui.actions}>
              <button className={ui.primary} disabled={reset.isPending || mismatch}>
                Set password
              </button>
            </div>
          </>
        )}
        {toLogin}
      </form>
    </main>
  );
}
