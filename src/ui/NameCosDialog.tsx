import { useState } from "react";
import { api } from "../api";
import { Dialog, Portrait } from "./Dialog";

/** Asked on first start, so the chief of staff isn't stuck with the default name. */
export function NameCosDialog({ name, onClose }: { name: string; onClose: () => void }) {
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const save = async () => {
    setBusy(true);
    try {
      await api.renameCos(value);
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };
  return (
    <Dialog
      label="Name your chief of staff"
      onClose={onClose}
      head={<Portrait who={{ name, role: "cos" }} size={52} tone="green" />}
      title="What should your chief of staff be called?"
      sub={<span>They sit at reception, keep track of what needs you, and send anything you hand over to the right room.</span>}
    >
      <form
        className="name-row"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <input
          autoFocus
          value={value}
          maxLength={20}
          placeholder="Their name"
          aria-label="Chief of staff's name"
          onChange={(e) => {
            setValue(e.target.value);
            setError(null);
          }}
        />
        <button className="btn primary" type="submit" disabled={busy || !value.trim()}>
          Save
        </button>
      </form>
      {error ? <p className="error">{error}</p> : <p className="quiet">You can change it later by clicking their name.</p>}
    </Dialog>
  );
}
