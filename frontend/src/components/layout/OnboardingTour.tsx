import { readStorageValue, writeStorageValue } from "../../lib/storage";
import { createContext, useContext, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  CompassIcon,
  HashIcon,
  MessageSquareIcon,
  BellIcon,
  SettingsIcon,
  LinkIcon,
} from "lucide-react";
import { Button } from "../ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "../ui/dialog";

const TourContext = createContext<(() => void) | null>(null);
const steps = ["welcome", "channels", "messages", "notifications", "slack", "settings"] as const;
const icons = [CompassIcon, HashIcon, MessageSquareIcon, BellIcon, LinkIcon, SettingsIcon];

export function OnboardingTour({
  open,
  onClose,
  onOpenSlack,
}: {
  open: boolean;
  onClose: () => void;
  onOpenSlack?: () => void;
}) {
  const { t } = useTranslation();
  const [step, setStep] = useState(0);
  const Icon = icons[step];
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <Icon
            aria-hidden="true"
            className="mb-2 size-10 rounded-xl bg-brand-muted p-2 text-brand-muted-foreground"
          />
          <p role="status" className="text-xs text-muted-foreground">
            {t("onboarding.progress", { current: step + 1, total: steps.length })}
          </p>
          <DialogTitle>{t(`onboarding.${steps[step]}.title`)}</DialogTitle>
          <DialogDescription className="leading-relaxed">
            {t(`onboarding.${steps[step]}.description`)}
          </DialogDescription>
        </DialogHeader>
        {steps[step] === "slack" && onOpenSlack && (
          <Button
            variant="outline"
            onClick={() => {
              onClose();
              onOpenSlack();
            }}
          >
            <LinkIcon aria-hidden="true" />
            {t("onboarding.openSlack")}
          </Button>
        )}
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t("onboarding.skip")}
          </Button>
          {step > 0 && (
            <Button variant="outline" onClick={() => setStep(step - 1)}>
              {t("onboarding.back")}
            </Button>
          )}
          <Button onClick={() => (step === steps.length - 1 ? onClose() : setStep(step + 1))}>
            {t(step === steps.length - 1 ? "onboarding.finish" : "onboarding.next")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Remounted by account ID so neither progress nor dismissal leaks across accounts. */
export function OnboardingProvider({
  userId,
  children,
  onOpenSlack,
}: {
  userId: string | null;
  children: ReactNode;
  onOpenSlack?: () => void;
}) {
  const storageKey = `zoo:onboarding:v1:${userId}`;
  const [open, setOpen] = useState(() => {
    if (!userId) return false;
    return readStorageValue(storageKey) !== "done";
  });
  const [run, setRun] = useState(0);
  const close = () => {
    setOpen(false);
    if (userId) {
      writeStorageValue(storageKey, "done");
    }
  };
  return (
    <TourContext.Provider
      value={() => {
        setRun((value) => value + 1);
        setOpen(true);
      }}
    >
      {children}
      <OnboardingTour key={run} open={open} onClose={close} onOpenSlack={onOpenSlack} />
    </TourContext.Provider>
  );
}

export function OnboardingButton() {
  const { t } = useTranslation();
  const start = useContext(TourContext);
  if (!start) return null;
  return (
    <Button
      variant="ghost"
      size="icon"
      className="size-10 rounded-xl text-muted-foreground"
      onClick={start}
      aria-label={t("onboarding.replay")}
      title={t("onboarding.replay")}
    >
      <CompassIcon aria-hidden="true" className="size-5" />
    </Button>
  );
}
