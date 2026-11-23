import { type ReactNode, useEffect, useRef } from "react";
import ui from "./ui.module.css";

/** A centred card over the page, which is dimmed and blurred behind it. Esc, the close button
 *  and a click beside the card close it, unless `locked` (while something is being saved). */
export function Modal({
  title,
  lead,
  onClose,
  locked = false,
  children,
}: {
  title: string;
  lead?: ReactNode;
  onClose: () => void;
  locked?: boolean;
  children: ReactNode;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const element = dialog.current;
    if (element && !element.open) element.showModal();
    return () => element?.close();
  }, []);

  return (
    <dialog
      ref={dialog}
      className={ui.modal}
      aria-labelledby="modal-title"
      onCancel={(event) => {
        event.preventDefault();
        if (!locked) onClose();
      }}
      onMouseDown={(event) => {
        // Only a press on the backdrop itself, not one that starts inside the card.
        if (event.target === dialog.current && !locked) onClose();
      }}
    >
      <div className={ui.modalCard}>
        <button
          type="button"
          className={ui.modalClose}
          onClick={onClose}
          disabled={locked}
          aria-label="Close"
        >
          ×
        </button>
        <p className={ui.modalBrand} aria-hidden="true">
          Kura
        </p>
        <h2 id="modal-title">{title}</h2>
        {lead && <p className={ui.modalLead}>{lead}</p>}
        {children}
      </div>
    </dialog>
  );
}

/** "Are you sure?" in a modal: the action button is red when it destroys something. */
export function Confirm({
  title,
  text,
  action,
  danger = false,
  pending = false,
  error,
  onConfirm,
  onClose,
}: {
  title: string;
  text: ReactNode;
  action: string;
  danger?: boolean;
  pending?: boolean;
  error?: string;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <Modal title={title} lead={text} onClose={onClose} locked={pending}>
      {error && (
        <p className={ui.error} role="alert">
          {error}
        </p>
      )}
      <div className={ui.modalActions}>
        <button
          type="button"
          className={danger ? ui.dangerSolid : ui.primary}
          disabled={pending}
          onClick={onConfirm}
          autoFocus
        >
          {action}
        </button>
        <button type="button" className={ui.ghost} disabled={pending} onClick={onClose}>
          Cancel
        </button>
      </div>
    </Modal>
  );
}
