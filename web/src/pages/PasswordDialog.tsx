import { useMutation } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { api } from "../api";
import ui from "../ui.module.css";

/** Changes the password of the account that is logged in. Other devices are logged out. */
export function PasswordDialog({ onClose }: { onClose: () => void }) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [again, setAgain] = useState("");
  const change = useMutation({
    mutationFn: () => api("/account/password", "PUT", { current, new: next }),
  });
  const mismatch = again !== "" && again !== next;

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    if (!mismatch) change.mutate();
  }

  return (
    <dialog
      ref={(element) => {
        if (element && !element.open) element.showModal();
      }}
      className={ui.dialog}
      onCancel={onClose}
      aria-labelledby="password-title"
    >
      <form onSubmit={onSubmit}>
        <h2 id="password-title">Change password</h2>
        {change.isSuccess ? (
          <>
            <p>Your password is changed. Other devices with this account are logged out.</p>
            <div className={ui.actions}>
              <button type="button" className={ui.primary} onClick={onClose} autoFocus>
                Done
              </button>
            </div>
          </>
        ) : (
          <>
            <p>Other devices with this account are logged out afterwards.</p>
            <label className={ui.field}>
              Current password
              <input
                className={ui.input}
                type="password"
                autoComplete="current-password"
                value={current}
                onChange={(event) => setCurrent(event.target.value)}
                required
                autoFocus
              />
            </label>
            <label className={ui.field}>
              New password
              <input
                className={ui.input}
                type="password"
                autoComplete="new-password"
                minLength={8}
                value={next}
                onChange={(event) => setNext(event.target.value)}
                required
              />
            </label>
            <label className={ui.field}>
              New password again
              <input
                className={ui.input}
                type="password"
                autoComplete="new-password"
                value={again}
                onChange={(event) => setAgain(event.target.value)}
                aria-invalid={mismatch}
                required
              />
            </label>
            {mismatch && <p className={ui.error}>The new passwords do not match.</p>}
            {change.isError && (
              <p className={ui.error} role="alert">
                {change.error.message}
              </p>
            )}
            <div className={ui.actions}>
              <button type="button" className={ui.ghost} onClick={onClose}>
                Cancel
              </button>
              <button className={ui.primary} disabled={change.isPending || mismatch}>
                Change password
              </button>
            </div>
          </>
        )}
      </form>
    </dialog>
  );
}
