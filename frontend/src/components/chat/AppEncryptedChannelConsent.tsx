import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { LockIcon } from "lucide-react";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../ui/dialog";

/**
 * Explicit consent before an App (bot) joins an encrypted channel as an E2EE device (#366,
 * docs/e2ee.md「Bot の端末」). Only Apps with the `e2ee:device` scope can join; the bot reads
 * messages sent after it joins, and members can remove it at any time.
 */
export function AppEncryptedChannelConsent({
  open,
  appName,
  channelName,
  supported,
  pending = false,
  error,
  onConfirm,
  onOpenChange,
}: {
  open: boolean;
  appName: string;
  channelName: string;
  /** Whether the App declares the `e2ee:device` scope. */
  supported: boolean;
  pending?: boolean;
  error?: string;
  onConfirm: () => void | Promise<void>;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const [agreed, setAgreed] = useState(false);
  const checkboxId = useId();
  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        if (pending) return;
        if (!value) setAgreed(false);
        onOpenChange(value);
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <LockIcon className="size-4" aria-hidden />
            {t("apps.e2ee.title", { app: appName, channel: channelName })}
          </DialogTitle>
          <DialogDescription>
            {supported ? t("apps.e2ee.description") : t("apps.e2ee.unsupported")}
          </DialogDescription>
        </DialogHeader>
        {supported ? (
          <>
            <ul className="list-disc space-y-1 pl-5 text-sm">
              <li>{t("apps.e2ee.readsAfterJoin", { app: appName })}</li>
              <li>{t("apps.e2ee.noHistory")}</li>
              <li>{t("apps.e2ee.trustOperator")}</li>
              <li>{t("apps.e2ee.remove")}</li>
            </ul>
            <label
              htmlFor={checkboxId}
              className="flex min-h-11 items-center gap-2 text-sm font-medium"
            >
              <Input
                id={checkboxId}
                type="checkbox"
                checked={agreed}
                disabled={pending}
                onChange={(event) => setAgreed(event.target.checked)}
              />
              {t("apps.e2ee.agree", { app: appName })}
            </label>
          </>
        ) : null}
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        ) : null}
        <DialogFooter>
          <Button variant="outline" disabled={pending} onClick={() => onOpenChange(false)}>
            {supported ? t("common.cancel") : t("apps.close")}
          </Button>
          {supported ? (
            <Button disabled={!agreed || pending} onClick={() => void onConfirm()}>
              {pending ? t("apps.e2ee.adding") : t("apps.e2ee.confirm")}
            </Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
