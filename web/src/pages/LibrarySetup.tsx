import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { api, type Settings, type Site, settingsQuery, siteNames, sites } from "../api";
import ui from "../ui.module.css";
import styles from "./LibrarySetup.module.css";

/** Walks an admin through creating the first library, the folder downloads go to. */
export function LibrarySetup({ suggestedPath }: { suggestedPath: string | null }) {
  const client = useQueryClient();
  const settings = useQuery(settingsQuery);
  const [step, setStep] = useState<1 | 2>(1);
  const [name, setName] = useState("Series");
  const [path, setPath] = useState(suggestedPath ?? "");
  const [defaultFor, setDefaultFor] = useState<Site[]>([...sites]);

  const create = useMutation({
    mutationFn: (current: Settings) =>
      api<Settings>("/settings", "PUT", {
        ...current,
        libraries: [{ name: name.trim(), path: path.trim(), default_for: defaultFor }],
      }),
    onSuccess: (saved) => {
      client.setQueryData(settingsQuery.queryKey, saved);
      client.invalidateQueries({ queryKey: ["status"] });
    },
  });

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    if (step === 1) setStep(2);
    else if (settings.data) create.mutate(settings.data);
  }

  return (
    <form className={styles.card} onSubmit={onSubmit}>
      <p className={styles.step}>Step {step} of 2</p>
      {step === 1 ? (
        <>
          <h1>Set up your library</h1>
          <p className={ui.lead}>
            A library is a folder on the server that downloads go to. Inside it, Kura stores
            files as Series/Title/Season XX, which Jellyfin reads as it is. You can add more
            libraries later in the settings.
          </p>
          <label className={ui.field}>
            Name
            <input
              className={ui.input}
              value={name}
              onChange={(event) => setName(event.target.value)}
              required
              autoFocus
            />
          </label>
          <label className={ui.field}>
            Folder on the server
            <input
              className={ui.input}
              value={path}
              onChange={(event) => setPath(event.target.value)}
              placeholder="/mnt/nas/Serien"
              pattern="/.*"
              title="A path starting at the root, like /mnt/nas/Serien"
              required
            />
          </label>
          <p className={styles.hint}>Kura creates the folder if it does not exist yet.</p>
        </>
      ) : (
        <>
          <h1>Which sites go here?</h1>
          <p className={ui.lead}>
            Downloads from these sites go to “{name.trim()}” unless you pick another library on a
            title's page. It is also your main library, used whenever no other one applies.
          </p>
          <div className={styles.sites}>
            {sites.map((site) => (
              <label key={site} className={ui.check}>
                <input
                  type="checkbox"
                  checked={defaultFor.includes(site)}
                  onChange={(event) =>
                    setDefaultFor(
                      event.target.checked
                        ? [...defaultFor, site]
                        : defaultFor.filter((other) => other !== site),
                    )
                  }
                />
                {siteNames[site]}
              </label>
            ))}
          </div>
        </>
      )}

      {create.isError && (
        <p className={ui.error} role="alert">
          {create.error.message}
        </p>
      )}
      <div className={`${ui.actions} ${styles.actions}`}>
        {step === 2 && (
          <button type="button" className={ui.ghost} onClick={() => setStep(1)}>
            Back
          </button>
        )}
        <button className={ui.primary} disabled={create.isPending || !settings.data}>
          {step === 1 ? "Next" : "Create library"}
        </button>
      </div>
    </form>
  );
}
