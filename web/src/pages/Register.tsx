import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { useNavigate } from "react-router";
import { api, type Role } from "../api";
import ui from "../ui.module.css";
import styles from "./Auth.module.css";

/** Where an invite link leads. Without a valid invite there is nothing here: nobody signs up
 *  on their own. The token is read from the part after `#` and taken out of the address bar. */
export function Register() {
  const client = useQueryClient();
  const navigate = useNavigate();
  const [token] = useState(() => {
    const token = new URLSearchParams(window.location.hash.slice(1)).get("token") ?? "";
    if (window.location.hash) window.history.replaceState(null, "", window.location.pathname);
    return token;
  });
  const invite = useQuery({
    queryKey: ["invite", token],
    queryFn: () => api<{ email: string; role: Role }>("/register/check", "POST", { token }),
    enabled: token !== "",
    retry: false,
    staleTime: Number.POSITIVE_INFINITY,
  });
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [again, setAgain] = useState("");
  const mismatch = again !== "" && again !== password;
  const register = useMutation({
    mutationFn: () => api("/register", "POST", { token, name, password }),
    onSuccess: () => {
      // Logged in now: the app starts with the new account.
      client.resetQueries();
      navigate("/", { replace: true });
    },
  });

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    if (!mismatch) register.mutate();
  }

  const back = (
    <p className={styles.aside}>
      <button type="button" className={styles.link} onClick={() => navigate("/", { replace: true })}>
        To the login
      </button>
    </p>
  );

  if (!token || invite.isError) {
    return (
      <main className={styles.page}>
        <div className={styles.form}>
          <h1>Kura</h1>
          <p className={ui.lead}>
            {invite.error
              ? invite.error.message.charAt(0).toUpperCase() + invite.error.message.slice(1) + "."
              : "This page only opens with an invite link. Accounts are created by an admin."}
          </p>
          {back}
        </div>
      </main>
    );
  }
  if (!invite.data) return null;

  return (
    <main className={styles.page}>
      <form className={styles.form} onSubmit={onSubmit}>
        <h1>Kura</h1>
        <p className={ui.lead}>
          You are invited{invite.data.role === "admin" ? " as an admin" : ""}. Choose a name and a
          password for your account.
        </p>
        <label className={ui.field}>
          Email
          <input className={ui.input} value={invite.data.email} readOnly disabled />
        </label>
        <label className={ui.field}>
          Name
          <input
            className={ui.input}
            value={name}
            onChange={(event) => setName(event.target.value)}
            autoComplete="username"
            required
            autoFocus
          />
        </label>
        <label className={ui.field}>
          Password
          <input
            className={ui.input}
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            autoComplete="new-password"
            minLength={8}
            required
          />
        </label>
        <label className={ui.field}>
          Password again
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
            The passwords differ.
          </p>
        )}
        {register.isError && (
          <p className={ui.error} role="alert">
            {register.error.message}
          </p>
        )}
        <div className={ui.actions}>
          <button className={ui.primary} disabled={register.isPending || mismatch}>
            Create account
          </button>
        </div>
        {back}
      </form>
    </main>
  );
}
