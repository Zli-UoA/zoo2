import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import type { SipDirectory, SipLine, SipLineInput, SipState } from "../../lib/sip/types";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Select } from "../ui/select";
import { ConfirmActionDialog } from "../ui/confirm-action-dialog";
import { SipControls, type SipControlsProps } from "./SipControls";

export interface SipPanelViewProps extends Omit<SipControlsProps, "state"> {
  directory: SipDirectory;
  state: SipState;
  loading: boolean;
  loadError: boolean;
  isAdmin: boolean;
  channelBusy: boolean;
  users: { id: string; name: string }[];
  onRefresh(this: void): void;
  onConnect(this: void, id: string): void;
  onDisconnect(this: void): void;
  onDial(this: void, number: string): void;
  onSave(this: void, id: string | null, input: SipLineInput): Promise<void>;
  onRemove(this: void, id: string): Promise<void>;
}
const emptyInput = (): SipLineInput => ({
  name: "",
  shared: false,
  websocket_path: "/sip/ws",
  sip_domain: "",
  username: "",
  password: "",
  phone_number: "",
  member_ids: [],
  phones: [],
});
export function SipPanelView(props: SipPanelViewProps) {
  const { t } = useTranslation();
  const { directory, state, loading, loadError } = props;
  const [selection, setSelection] = useState("");
  const [number, setNumber] = useState("");
  const [editing, setEditing] = useState<{ id: string | null; input: SipLineInput } | null>(null);
  const [deleting, setDeleting] = useState<SipLine | null>(null);
  const [pending, setPending] = useState(false);
  const [writeError, setWriteError] = useState(false);
  const selected = directory.lines.find((line) => line.id === (state.lineId ?? selection));
  const patch = (update: Partial<SipLineInput>) =>
    setEditing((current) =>
      current ? { ...current, input: { ...current.input, ...update } } : null,
    );
  const open = (line?: SipLine) => {
    setWriteError(false);
    setEditing({
      id: line?.id ?? null,
      input: line
        ? {
            name: line.name,
            shared: line.shared,
            websocket_path: line.websocket_path,
            sip_domain: line.sip_domain,
            username: line.username,
            password: "",
            phone_number: line.phone_number,
            member_ids: [...line.member_ids],
            phones: line.phones.map((p) => ({ ...p })),
          }
        : emptyInput(),
    });
  };
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!editing || pending) return;
    setPending(true);
    setWriteError(false);
    try {
      await props.onSave(editing.id, editing.input);
      setEditing(null);
    } catch {
      setWriteError(true);
    } finally {
      setPending(false);
    }
  }
  return (
    <div className="@container space-y-5">
      <p className="text-sm text-muted-foreground">{t("sip.description")}</p>
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" onClick={props.onRefresh} disabled={loading}>
          {t("sip.refresh")}
        </Button>
        {directory.enabled ? (
          <Button type="button" onClick={() => open()} disabled={pending}>
            {t("sip.addLine")}
          </Button>
        ) : null}
      </div>
      {loading ? <p role="status">{t("sip.loading")}</p> : null}
      {loadError ? (
        <p role="alert" className="text-destructive">
          {t("sip.loadError")}
        </p>
      ) : null}
      {!loading && !loadError && !directory.enabled ? <p>{t("sip.disabled")}</p> : null}
      {directory.enabled ? (
        <>
          {props.channelBusy ? <p role="status">{t("sip.channelBusy")}</p> : null}
          <div className="space-y-3 rounded-lg border p-4">
            <label className="grid gap-1 text-sm">
              {t("sip.line")}
              <Select
                className="w-full"
                value={state.lineId ?? selection}
                disabled={state.status !== "offline"}
                onChange={(e) => setSelection(e.target.value)}
              >
                <option value="">{t("sip.selectLine")}</option>
                {directory.lines
                  .filter((line) => line.can_connect)
                  .map((line) => (
                    <option key={line.id} value={line.id}>
                      {line.name} · {t(line.shared ? "sip.shared" : "sip.personal")}
                      {line.phone_number ? ` · ${line.phone_number}` : ""}
                    </option>
                  ))}
              </Select>
            </label>
            {state.status === "offline" ? (
              <Button
                disabled={!selected?.can_connect || props.channelBusy || loading || loadError}
                onClick={() => {
                  if (selected) props.onConnect(selected.id);
                }}
              >
                {t("sip.connect")}
              </Button>
            ) : (
              <Button type="button" variant="outline" onClick={props.onDisconnect}>
                {t("sip.disconnect")}
              </Button>
            )}
            <SipControls
              state={state}
              onAnswer={props.onAnswer}
              onHangup={props.onHangup}
              onMute={props.onMute}
              onDtmf={props.onDtmf}
            />
            <form
              className="flex flex-wrap items-end gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                props.onDial(number.trim());
              }}
            >
              <label className="grid min-w-0 flex-1 gap-1 text-sm">
                {t("sip.number")}
                <Input
                  type="tel"
                  value={number}
                  onChange={(e) => setNumber(e.target.value)}
                  maxLength={33}
                  pattern="\+?[0-9]{1,32}"
                  required
                />
              </label>
              <Button
                type="submit"
                disabled={
                  state.status !== "ready" || state.busy || !/^\+?\d{1,32}$/.test(number.trim())
                }
              >
                {t("sip.dial")}
              </Button>
            </form>
          </div>
          {!directory.lines.length ? <p>{t("sip.empty")}</p> : null}
          <ul className="space-y-3">
            {directory.lines.map((line) => (
              <li key={line.id} className="space-y-3 rounded-lg border p-4">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="min-w-0">
                    <h3 className="break-words font-medium">{line.name}</h3>
                    <p className="break-all text-sm text-muted-foreground">
                      {t(line.shared ? "sip.shared" : "sip.personal")}
                      {line.phone_number ? ` · ${line.phone_number}` : ""}
                    </p>
                  </div>
                  {line.can_manage ? (
                    <div className="flex gap-2">
                      <Button
                        type="button"
                        variant="outline"
                        disabled={pending}
                        onClick={() => open(line)}
                      >
                        {t("sip.edit")}
                      </Button>
                      <Button
                        variant="destructive"
                        disabled={pending}
                        onClick={() => {
                          setWriteError(false);
                          setDeleting(line);
                        }}
                      >
                        {t("sip.delete")}
                      </Button>
                    </div>
                  ) : null}
                </div>
                {line.phones.length ? (
                  <ul className="space-y-1">
                    {line.phones.map((phone, index) => (
                      <li
                        key={index}
                        className="flex flex-wrap items-center justify-between gap-2 text-sm"
                      >
                        <span className="min-w-0 break-words">
                          {phone.name} · {phone.extension}
                        </span>
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={
                            state.lineId !== line.id || state.status !== "ready" || state.busy
                          }
                          onClick={() => props.onDial(phone.extension)}
                        >
                          {t("sip.dial")}
                        </Button>
                      </li>
                    ))}
                  </ul>
                ) : null}
              </li>
            ))}
          </ul>
        </>
      ) : null}
      {editing ? (
        <form
          onSubmit={(e) => void save(e)}
          className="space-y-4 rounded-lg border p-4"
          aria-label={t("sip.lineForm")}
        >
          <h3 className="font-medium">{t(editing.id ? "sip.edit" : "sip.addLine")}</h3>
          <fieldset disabled={pending} className="space-y-4">
            <label className="grid gap-1 text-sm">
              {t("sip.name")}
              <Input
                required
                value={editing.input.name}
                maxLength={100}
                onChange={(e) => patch({ name: e.target.value })}
              />
            </label>
            <label className="grid gap-1 text-sm">
              {t("sip.ownership")}
              <Select
                value={editing.input.shared ? "shared" : "personal"}
                disabled={!!editing.id}
                onChange={(e) => patch({ shared: e.target.value === "shared", member_ids: [] })}
              >
                <option value="personal">{t("sip.personal")}</option>
                {props.isAdmin || editing.input.shared ? (
                  <option value="shared">{t("sip.shared")}</option>
                ) : null}
              </Select>
            </label>
            {(["phone_number", "websocket_path", "sip_domain", "username"] as const).map((key) => (
              <label key={key} className="grid gap-1 text-sm">
                {t(`sip.${key}`)}
                <Input
                  value={editing.input[key]}
                  required={key !== "phone_number"}
                  maxLength={key === "websocket_path" ? 200 : 128}
                  onChange={(e) => patch({ [key]: e.target.value })}
                />
              </label>
            ))}
            <label className="grid gap-1 text-sm">
              {t("sip.password")}
              <Input
                type="password"
                value={editing.input.password ?? ""}
                required={!editing.id}
                maxLength={1024}
                placeholder={editing.id ? t("sip.keepPassword") : undefined}
                onChange={(e) => patch({ password: e.target.value })}
              />
            </label>
            {editing.input.shared ? (
              <fieldset className="space-y-2">
                <legend className="text-sm font-medium">{t("sip.members")}</legend>
                <p className="text-xs text-muted-foreground">{t("sip.membersHelp")}</p>
                <div className="max-h-48 space-y-1 overflow-auto">
                  {props.users.map((user) => (
                    <label key={user.id} className="flex items-center gap-2 text-sm">
                      <input
                        type="checkbox"
                        checked={editing.input.member_ids.includes(user.id)}
                        onChange={(e) =>
                          patch({
                            member_ids: e.target.checked
                              ? [...editing.input.member_ids, user.id]
                              : editing.input.member_ids.filter((id) => id !== user.id),
                          })
                        }
                      />
                      <span className="break-words">{user.name}</span>
                    </label>
                  ))}
                  {editing.input.member_ids
                    .filter((id) => !props.users.some((user) => user.id === id))
                    .map((id) => (
                      <label key={id} className="flex items-start gap-2 text-sm">
                        <input
                          type="checkbox"
                          checked
                          onChange={() =>
                            patch({
                              member_ids: editing.input.member_ids.filter(
                                (member) => member !== id,
                              ),
                            })
                          }
                        />
                        <span className="min-w-0 break-all">
                          {t("sip.unavailableMember")} · {id}
                        </span>
                      </label>
                    ))}
                </div>
              </fieldset>
            ) : null}
            <fieldset className="space-y-3">
              <legend className="text-sm font-medium">{t("sip.phones")}</legend>
              <p className="text-xs text-muted-foreground">{t("sip.phonesHelp")}</p>
              {editing.input.phones.map((phone, index) => (
                <div
                  key={index}
                  className="grid grid-cols-1 gap-2 rounded border p-2 @lg:grid-cols-[minmax(0,1fr)_8rem_auto]"
                >
                  <label className="grid min-w-0 gap-1 text-sm">
                    {t("sip.phoneName")}
                    <Input
                      required
                      maxLength={100}
                      value={phone.name}
                      onChange={(e) =>
                        patch({
                          phones: editing.input.phones.map((p, i) =>
                            i === index ? { ...p, name: e.target.value } : p,
                          ),
                        })
                      }
                    />
                  </label>
                  <label className="grid gap-1 text-sm">
                    {t("sip.extension")}
                    <Input
                      required
                      type="tel"
                      className="w-full"
                      maxLength={33}
                      pattern="\+?[0-9]{1,32}"
                      value={phone.extension}
                      onChange={(e) =>
                        patch({
                          phones: editing.input.phones.map((p, i) =>
                            i === index ? { ...p, extension: e.target.value } : p,
                          ),
                        })
                      }
                    />
                  </label>
                  <Button
                    variant="outline"
                    onClick={() =>
                      patch({ phones: editing.input.phones.filter((_, i) => i !== index) })
                    }
                  >
                    {t("sip.removePhone")}
                  </Button>
                </div>
              ))}
              <Button
                variant="outline"
                disabled={editing.input.phones.length >= 100}
                onClick={() =>
                  patch({ phones: [...editing.input.phones, { name: "", extension: "" }] })
                }
              >
                {t("sip.addPhone")}
              </Button>
            </fieldset>
            <p className="text-xs text-muted-foreground">{t("sip.pbxHelp")}</p>
            {writeError ? (
              <p role="alert" className="text-destructive">
                {t("sip.writeError")}
              </p>
            ) : null}
            <div className="flex gap-2">
              <Button type="submit">{t(pending ? "sip.saving" : "sip.save")}</Button>
              <Button type="button" variant="outline" onClick={() => setEditing(null)}>
                {t("sip.cancel")}
              </Button>
            </div>
          </fieldset>
        </form>
      ) : null}
      <ConfirmActionDialog
        open={!!deleting}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
        title={t("sip.deleteTitle")}
        description={t("sip.deleteHelp")}
        confirmLabel={t("sip.delete")}
        pending={pending}
        error={writeError ? t("sip.writeError") : undefined}
        onConfirm={async () => {
          if (!deleting || pending) return;
          setPending(true);
          setWriteError(false);
          try {
            await props.onRemove(deleting.id);
            setDeleting(null);
          } catch {
            setWriteError(true);
          } finally {
            setPending(false);
          }
        }}
      />
    </div>
  );
}
