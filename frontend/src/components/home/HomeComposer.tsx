import { ActivityPubText, type CustomEmoji } from "./ActivityPubEmoji";
import { flushSync } from "react-dom";
import { useImperativeHandle, useRef, useState, type Ref } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowLeftIcon,
  GlobeIcon,
  HomeIcon,
  LockKeyholeIcon,
  NetworkIcon,
  UnplugIcon,
  SendIcon,
  SquarePenIcon,
  LoaderCircleIcon,
  XIcon,
} from "lucide-react";
import { useActiveChannels, useMyChannelMemberships } from "../../hooks/useRealtime";
import { useVisualViewport } from "../layout/useVisualViewport";
import { useMediaQuery } from "../layout/useMediaQuery";
import { Button } from "../ui/button";
import { Textarea } from "../ui/textarea";
import { Select } from "../ui/select";
import { Dialog, DialogContent, DialogTitle, DialogTrigger } from "../ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "../ui/dropdown-menu";

export interface HomePostInput {
  content: string;
  apub: boolean;
  visibility: "public" | "unlisted" | "followers";
  channel_id: string | null;
  in_reply_to?: string;
}

export interface ReplyTarget {
  emojis?: CustomEmoji[];
  actor_emojis?: CustomEmoji[];
  id: string;
  display_name: string;
  content: string;
  visibility?: string;
  apub?: boolean;
  remote?: boolean;
}
export interface HomeComposerHandle {
  reply: (post: ReplyTarget) => void;
}
/** One draft and one form, shown as a desktop dialog or a mobile screen. */
export function HomeComposer({
  federationEnabled,
  onPost,
  ref,
}: {
  federationEnabled: boolean;
  onPost: (post: HomePostInput) => Promise<void>;
  ref?: Ref<HomeComposerHandle>;
}) {
  const { t } = useTranslation();
  const isDesktop = useMediaQuery("(min-width: 768px)");
  const channelsQuery = useActiveChannels();
  const { joinedChannelIds } = useMyChannelMemberships();
  const [open, setOpen] = useState(false);
  const viewportRef = useVisualViewport(open && !isDesktop);
  const [replyTo, setReplyTo] = useState<ReplyTarget | null>(null);
  const [content, setContent] = useState("");
  const [visibility, setVisibility] = useState<HomePostInput["visibility"]>("unlisted");
  const [federatePreference, setFederatePreference] = useState<boolean | null>(null);
  const [channelId, setChannelId] = useState("");
  const [sending, setSending] = useState(false);
  const [failed, setFailed] = useState(false);
  const inFlight = useRef(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const federated = federationEnabled && replyTo?.apub !== false && (federatePreference ?? true);
  const length = Array.from(content).length;
  const AudienceIcon =
    visibility === "public" ? GlobeIcon : visibility === "followers" ? LockKeyholeIcon : HomeIcon;
  const FederationIcon = federated ? NetworkIcon : UnplugIcon;
  const CloseIcon = isDesktop ? XIcon : ArrowLeftIcon;
  const options = [
    { value: "public", icon: GlobeIcon },
    { value: "unlisted", icon: HomeIcon },
    { value: "followers", icon: LockKeyholeIcon },
  ] as const;

  const submit = async () => {
    if (inFlight.current || !content.trim() || length > 1000) return;
    inFlight.current = true;
    setSending(true);
    setFailed(false);
    try {
      await onPost({
        content,
        apub: federated,
        visibility,
        channel_id: channelId || null,
        ...(replyTo ? { in_reply_to: replyTo.id } : {}),
      });
      setReplyTo(null);
      setContent("");
      setOpen(false);
    } catch {
      setFailed(true);
    } finally {
      inFlight.current = false;
      setSending(false);
    }
  };

  useImperativeHandle(
    ref,
    () => ({
      reply: (post) => {
        if (inFlight.current) return;
        const show = () => {
          setReplyTo(post);
          setChannelId("");
          setVisibility(
            post.visibility === "followers"
              ? "followers"
              : post.visibility === "public"
                ? "public"
                : "unlisted",
          );
          setOpen(true);
        };
        if (!isDesktop) {
          flushSync(show);
          inputRef.current?.focus({ preventScroll: true });
        } else show();
      },
    }),
    [isDesktop],
  );

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (inFlight.current) return;
        if (next && !isDesktop) {
          // Mount and focus within the opening gesture so mobile browsers can show the IME.
          flushSync(() => setOpen(true));
          inputRef.current?.focus({ preventScroll: true });
        } else setOpen(next);
      }}
    >
      <DialogTrigger
        render={
          <Button
            size="icon-lg"
            className="absolute right-5 bottom-[max(1.25rem,env(safe-area-inset-bottom))] z-20 size-14 rounded-full shadow-lg transition-transform hover:-translate-y-0.5 md:right-8 md:bottom-8"
            aria-label={t("home.openComposer")}
            title={t("home.openComposer")}
          />
        }
      >
        <SquarePenIcon className="size-6" aria-hidden="true" />
      </DialogTrigger>
      <DialogContent
        ref={viewportRef}
        showCloseButton={false}
        initialFocus={isDesktop ? inputRef : false}
        className="flex flex-col gap-0 overflow-hidden p-0 max-md:top-0 max-md:h-dvh max-md:animate-none sm:max-w-xl"
        style={
          isDesktop
            ? undefined
            : {
                top: 0,
                left: 0,
                translate: "none",
                transform: "none",
                width: "100%",
                maxWidth: "none",
                height: "100dvh",
                maxHeight: "none",
                borderRadius: 0,
                animation: "none",
              }
        }
      >
        <form
          className="flex h-full min-h-0 flex-col"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <header className="flex shrink-0 items-center gap-2 border-b px-3 py-2 pt-[max(.5rem,env(safe-area-inset-top))]">
            <Button
              type="button"
              variant="ghost"
              size="icon"
              disabled={sending}
              aria-label={t("common.close")}
              title={t("common.close")}
              onClick={() => setOpen(false)}
            >
              <CloseIcon aria-hidden="true" />
            </Button>
            <DialogTitle className="min-w-0 flex-1 truncate text-sm">
              {t("home.compose")}
            </DialogTitle>
            <div
              role="group"
              aria-label={t("home.postOptions")}
              className="ml-auto flex shrink-0 items-center gap-1"
            >
              <DropdownMenu>
                <DropdownMenuTrigger
                  render={
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      disabled={sending}
                      aria-label={`${t("home.audience")}: ${t(`home.visibility.${visibility}`)}`}
                      title={`${t("home.audience")}: ${t(`home.visibility.${visibility}`)}`}
                    />
                  }
                >
                  <AudienceIcon aria-hidden="true" />
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuRadioGroup
                    value={visibility}
                    onValueChange={(value) => {
                      if (value !== "public" && value !== "unlisted" && value !== "followers")
                        return;
                      setVisibility(value);
                      if (value === "followers") setChannelId("");
                    }}
                  >
                    {options.map(({ value, icon: Icon }) => (
                      <DropdownMenuRadioItem
                        key={value}
                        value={value}
                        disabled={
                          (replyTo?.visibility === "followers" && value !== "followers") ||
                          (replyTo?.visibility === "unlisted" && value === "public")
                        }
                        closeOnClick
                      >
                        <Icon aria-hidden="true" />
                        {t(`home.visibility.${value}`)}
                      </DropdownMenuRadioItem>
                    ))}
                  </DropdownMenuRadioGroup>
                </DropdownMenuContent>
              </DropdownMenu>
              <DropdownMenu>
                <DropdownMenuTrigger
                  render={
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      disabled={sending}
                      aria-label={t(federated ? "home.federationOn" : "home.federationOff")}
                      title={t(federated ? "home.federationOn" : "home.federationOff")}
                    />
                  }
                >
                  <FederationIcon aria-hidden="true" />
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuRadioGroup
                    value={federated ? "on" : "off"}
                    onValueChange={(value) => setFederatePreference(value === "on")}
                  >
                    <DropdownMenuRadioItem
                      value="on"
                      disabled={!federationEnabled || replyTo?.apub === false}
                      closeOnClick
                    >
                      <NetworkIcon aria-hidden="true" />
                      {t("home.federationOn")}
                    </DropdownMenuRadioItem>
                    <DropdownMenuRadioItem value="off" closeOnClick>
                      <UnplugIcon aria-hidden="true" />
                      {t("home.federationOff")}
                    </DropdownMenuRadioItem>
                  </DropdownMenuRadioGroup>
                  {!federationEnabled ? (
                    <p className="max-w-60 px-2 py-1 text-xs text-muted-foreground">
                      {t("home.federationDisabled")}
                    </p>
                  ) : null}
                </DropdownMenuContent>
              </DropdownMenu>
              <Button
                type="submit"
                size="icon"
                disabled={sending || !content.trim() || length > 1000}
                aria-label={t(sending ? "home.sending" : "home.post")}
                title={t(sending ? "home.sending" : "home.post")}
              >
                {sending ? (
                  <LoaderCircleIcon className="animate-spin" aria-hidden="true" />
                ) : (
                  <SendIcon aria-hidden="true" />
                )}
              </Button>
            </div>
          </header>
          <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-4 pb-[max(1rem,env(safe-area-inset-bottom))]">
            {replyTo ? (
              <div className="flex items-start gap-2 rounded-lg bg-muted p-3 text-xs">
                <div className="min-w-0 flex-1">
                  <p className="font-medium">
                    <ActivityPubText
                      text={t("home.replyTo", { name: replyTo.display_name })}
                      emojis={replyTo.actor_emojis}
                    />
                  </p>
                  <p className="mt-1 line-clamp-2 break-words text-muted-foreground">
                    <ActivityPubText text={replyTo.content} emojis={replyTo.emojis} />
                  </p>
                </div>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  disabled={sending}
                  aria-label={t("home.cancelReply")}
                  onClick={() => setReplyTo(null)}
                >
                  <XIcon />
                </Button>
              </div>
            ) : null}
            <Textarea
              ref={inputRef}
              value={content}
              onChange={(e) => setContent(e.target.value)}
              disabled={sending}
              aria-label={t("home.compose")}
              aria-describedby="home-post-count"
              aria-invalid={length > 1000 || undefined}
              placeholder={t("home.placeholder")}
              className="min-h-40 flex-1 resize-none border-0 p-0 shadow-none focus-visible:ring-0 md:min-h-48"
            />
            <div className="flex flex-wrap items-center justify-between gap-2">
              <label className="flex min-w-0 items-center gap-2 text-xs">
                {t("home.channel")}
                <Select
                  aria-label={t("home.channel")}
                  value={channelId}
                  disabled={sending || visibility === "followers"}
                  onChange={(e) => setChannelId(e.target.value)}
                >
                  <option value="">{t("home.noChannel")}</option>
                  {(channelsQuery.data ?? [])
                    .filter(
                      (c) =>
                        !c.is_private && !c.is_dm && !c.archive_at && joinedChannelIds.has(c.id),
                    )
                    .map((c) => (
                      <option key={c.id} value={c.id}>
                        #{c.channel_name}
                      </option>
                    ))}
                </Select>
              </label>
              <span
                id="home-post-count"
                className={
                  length > 1000 ? "text-xs text-destructive" : "text-xs text-muted-foreground"
                }
              >
                {length} / 1000
              </span>
            </div>
            {failed ? (
              <p role="alert" className="text-sm text-destructive">
                {t("home.sendError")}
              </p>
            ) : null}
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
