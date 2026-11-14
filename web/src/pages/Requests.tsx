import { useMutation, useQuery } from "@tanstack/react-query";
import { Link } from "react-router";
import {
  api,
  describePicked,
  formatTime,
  hosterNames,
  languageLabel,
  type Request,
  type RequestState,
  requestsQuery,
  siteNames,
} from "../api";
import ui from "../ui.module.css";

const stateNames: Record<RequestState, string> = {
  open: "Waiting for an admin",
  accepted: "Accepted",
  rejected: "Rejected",
};
const stateStyles: Record<RequestState, string> = {
  open: ui.warn,
  accepted: ui.ok,
  rejected: ui.bad,
};

/** Admins decide on every request; users follow their own. */
export function Requests({ admin }: { admin: boolean }) {
  const requests = useQuery(requestsQuery);
  const decide = useMutation({
    mutationFn: ({ id, action }: { id: number; action: "accept" | "reject" | "cancel" }) =>
      api(`/requests/${id}/${action}`, "POST"),
  });
  const clear = useMutation({ mutationFn: () => api("/requests/clear", "POST") });
  const list = requests.data ?? [];
  const open = list.filter((request) => request.state === "open");
  const decided = list.filter((request) => request.state !== "open");

  return (
    <>
      <h1>Requests</h1>
      <p className={ui.lead}>
        {admin
          ? "Accepted requests go to the queue, into the site's default library."
          : "Ask for a title on its page. An admin accepts or rejects it."}
      </p>
      {(requests.error ?? decide.error ?? clear.error) && (
        <p className={ui.error} role="alert">
          {(requests.error ?? decide.error ?? clear.error)?.message}
        </p>
      )}
      {requests.isSuccess && list.length === 0 && (
        <p className={ui.notice}>{admin ? "No requests yet." : "You have not asked for anything yet."}</p>
      )}

      {open.length > 0 && (
        <>
          <h2 className={ui.section}>Open</h2>
          <ul className={ui.list}>
            {open.map((request) => (
              <Row key={request.id} request={request} admin={admin}>
                {!admin && (
                  <button
                    type="button"
                    className={ui.ghost}
                    disabled={decide.isPending}
                    aria-label={`Cancel the request for ${request.series_title}`}
                    onClick={() => decide.mutate({ id: request.id, action: "cancel" })}
                  >
                    Cancel
                  </button>
                )}
                {admin && (
                  <>
                    <button
                      type="button"
                      className={ui.ghost}
                      disabled={decide.isPending}
                      aria-label={`Reject ${request.series_title} for ${request.user_name}`}
                      onClick={() => decide.mutate({ id: request.id, action: "reject" })}
                    >
                      Reject
                    </button>
                    <button
                      type="button"
                      className={ui.primary}
                      disabled={decide.isPending}
                      aria-label={`Accept ${request.series_title} for ${request.user_name}`}
                      onClick={() => decide.mutate({ id: request.id, action: "accept" })}
                    >
                      Accept
                    </button>
                  </>
                )}
              </Row>
            ))}
          </ul>
        </>
      )}

      {decided.length > 0 && (
        <>
          <h2 className={ui.section}>Decided</h2>
          {admin && (
            <div className={ui.toolbar}>
              <button
                type="button"
                className={ui.button}
                disabled={clear.isPending}
                onClick={() => clear.mutate()}
              >
                Clear requests
              </button>
            </div>
          )}
          <ul className={ui.list}>
            {decided.map((request) => (
              <Row key={request.id} request={request} admin={admin} />
            ))}
          </ul>
        </>
      )}
    </>
  );
}

function Row({
  request,
  admin,
  children,
}: {
  request: Request;
  admin: boolean;
  children?: React.ReactNode;
}) {
  const details = [
    describePicked(request.picked),
    languageLabel(request.language),
    request.hoster && (hosterNames[request.hoster] ?? request.hoster),
    siteNames[request.site],
    admin && `by ${request.user_name}`,
    formatTime(request.created_at),
  ].filter(Boolean);

  return (
    <li className={ui.row}>
      <div className={ui.grow}>
        <Link to={`/series/${request.site}/${request.slug}`}>{request.series_title}</Link>
        <p className={`${ui.muted} ${ui.small}`}>{details.join(" · ")}</p>
      </div>
      {/* Admins act on open requests; everyone else sees where it stands. */}
      {/* Open requests: admins decide, the user who asked can still cancel. */}
      {!admin && <span className={stateStyles[request.state]}>{stateNames[request.state]}</span>}
      {request.state === "open" ? (
        children
      ) : (
        admin && <span className={stateStyles[request.state]}>{stateNames[request.state]}</span>
      )}
    </li>
  );
}
