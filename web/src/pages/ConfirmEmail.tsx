import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useNavigate } from "react-router";
import { accountQuery, api } from "../api";
import ui from "../ui.module.css";
import styles from "./Auth.module.css";

/** Where the link mailed to a new email address leads. Opening it makes the address the
 *  account's email, where reset links go; it works without being logged in on this device.
 *  The token is read from the part after `#` and taken out of the address bar. */
export function ConfirmEmail() {
  const client = useQueryClient();
  const navigate = useNavigate();
  const [token] = useState(() => {
    const token = new URLSearchParams(window.location.hash.slice(1)).get("token") ?? "";
    if (window.location.hash) window.history.replaceState(null, "", window.location.pathname);
    return token;
  });
  const confirm = useMutation({
    mutationFn: () => api("/account/email/confirm", "POST", { token }),
    onSuccess: () => client.invalidateQueries({ queryKey: accountQuery.queryKey }),
  });

  return (
    <main className={styles.page}>
      <div className={styles.form}>
        <h1>Kura</h1>
        {!token ? (
          <p className={ui.lead}>
            This link is incomplete. Enter the address again under Settings → Account for a new
            one.
          </p>
        ) : confirm.isSuccess ? (
          <p className={styles.sent} role="status">
            Your email is confirmed. Password reset links now go to this address.
          </p>
        ) : (
          <>
            <p className={ui.lead}>
              Confirm that this address is yours, so Kura sends your password reset links here.
            </p>
            {confirm.isError && (
              <p className={ui.error} role="alert">
                {confirm.error.message}
              </p>
            )}
            <div className={ui.actions}>
              <button
                type="button"
                className={ui.primary}
                disabled={confirm.isPending}
                onClick={() => confirm.mutate()}
              >
                Confirm email
              </button>
            </div>
          </>
        )}
        <p className={styles.aside}>
          <button
            type="button"
            className={styles.link}
            onClick={() => navigate("/", { replace: true })}
          >
            Open Kura
          </button>
        </p>
      </div>
    </main>
  );
}
