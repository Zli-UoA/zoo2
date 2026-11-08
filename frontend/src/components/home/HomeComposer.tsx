import {
  loadHomePostSettings,
  saveHomePostSettings,
  replyVisibility,
  type HomePostSettings,
} from "../../lib/homePreferences";
import { ActivityPubText, ActivityPubContent, type CustomEmoji } from "./ActivityPubEmoji";
import { flushSync } from "react-dom";
import { useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";
import { useTranslation } from "react-i18next";
import {
  ImagePlusIcon,
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

import { attachmentFilesFromClipboard, resolveAttachmentContentType } from "../../lib/attachments";
import { useCollections } from "../../lib/electric/collections-context";
import { createConfiguredApiClient } from "../../lib/api/client";

export interface HomePostInput {
  image_ids?: string[];
  content: string;
  apub: boolean;
  visibility: HomePostSettings["visibility"];
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
  channel_id?: string | null;
}
export interface HomeComposerHandle {
  reply: (post: ReplyTarget) => void;
}
/** One draft and one form, shown as a desktop dialog or a mobile screen. */
export function HomeComposer({
  settingsKey,
  federationEnabled,
  onPost,
  ref,
}: {
  settingsKey: string | null;
  federationEnabled: boolean;
  onPost: (post: HomePostInput) => Promise<void>;
  ref?: Ref<HomeComposerHandle>;
}) {
  const { t } = useTranslation();
  const { config } = useCollections();
  const discardImage = (id: string) => {
    void createConfiguredApiClient(config)
      .request(`/api/home/images/${encodeURIComponent(id)}`, { method: "DELETE" })
      .catch(() => {});
  };
  const [images, setImages] = useState<{ file: File; preview: string; id?: string }[]>([]);
  const imagesRef = useRef(images);
  imagesRef.current = images;
  const [uploading, setUploading] = useState(false);
  const uploadLock = useRef(false);
  const [imageError, setImageError] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const uploadController = useRef<AbortController | null>(null);
  useEffect(
    () => () => {
      uploadController.current?.abort();
      imagesRef.current.forEach((image) => {
        URL.revokeObjectURL(image.preview);
        if (image.id) discardImage(image.id);
      });
    },
    [],
  );
  const addImages = async (files: File[]) => {
    if (uploadLock.current || inFlight.current) return;
    const controller = new AbortController();
    uploadController.current = controller;
    uploadLock.current = true;
    setUploading(true);
    setImageError(false);
    try {
      for (const file of files) {
        if (controller.signal.aborted) return;
        const contentType = resolveAttachmentContentType(file);
        if (
          imagesRef.current.length >= 10 ||
          !["image/png", "image/jpeg", "image/gif", "image/webp"].includes(contentType ?? "") ||
          !file.size ||
          file.size > 8 * 1024 * 1024
        ) {
          setImageError(true);
          continue;
        }
        const draft = { file, preview: URL.createObjectURL(file) };
        imagesRef.current = [...imagesRef.current, draft];
        setImages(imagesRef.current);
        try {
          const data = await new Promise<string>((resolve, reject) => {
            const reader = new FileReader();
            reader.onload = () => {
              if (typeof reader.result === "string") resolve(reader.result.split(",")[1]!);
              else reject(new Error("Image read failed"));
            };
            reader.onerror = () => reject(reader.error);
            reader.readAsDataURL(file);
          });
          const result = await createConfiguredApiClient(config).request<{ id: string }>(
            "/api/home/images",
            {
              method: "POST",
              body: { name: file.name, content_type: contentType, data },
              signal: controller.signal,
            },
          );
          if (controller.signal.aborted) {
            discardImage(result.id);
            return;
          }
          imagesRef.current = imagesRef.current.map((image) =>
            image === draft ? { ...image, id: result.id } : image,
          );
          setImages(imagesRef.current);
        } catch {
          URL.revokeObjectURL(draft.preview);
          imagesRef.current = imagesRef.current.filter((image) => image !== draft);
          setImages(imagesRef.current);
          setImageError(true);
        }
      }
    } finally {
      uploadLock.current = false;
      setUploading(false);
    }
  };
  const isDesktop = useMediaQuery("(min-width: 768px)");
  const channelsQuery = useActiveChannels();
  const { joinedChannelIds, isInitialSyncLoading: membershipsSyncLoading } =
    useMyChannelMemberships();
  const [open, setOpen] = useState(false);
  const viewportRef = useVisualViewport(open && !isDesktop);
  const [replyTo, setReplyTo] = useState<ReplyTarget | null>(null);
  const [content, setContent] = useState("");
  const [previousSettings] = useState(() => loadHomePostSettings(settingsKey));
  const [visibility, setVisibility] = useState(previousSettings.visibility);
  const [federatePreference, setFederatePreference] = useState(previousSettings.apub);
  const [channelId, setChannelId] = useState(previousSettings.channel_id ?? "");
  const availableChannels = (channelsQuery.data ?? []).filter(
    (channel) =>
      !channel.is_private &&
      !channel.is_dm &&
      !channel.archive_at &&
      joinedChannelIds.has(channel.id),
  );
  const selectedChannelId =
    visibility !== "followers" && availableChannels.some((channel) => channel.id === channelId)
      ? channelId
      : "";
  const channelSelectionPending =
    channelId !== "" &&
    visibility !== "followers" &&
    !selectedChannelId &&
    (channelsQuery.isInitialSyncLoading || membershipsSyncLoading);
  useEffect(() => {
    if (!channelId || visibility === "followers" || selectedChannelId || channelSelectionPending)
      return;
    const saved = loadHomePostSettings(settingsKey);
    if (saved.channel_id === channelId)
      saveHomePostSettings(settingsKey, { ...saved, channel_id: null });
    setChannelId("");
  }, [channelId, visibility, selectedChannelId, channelSelectionPending, settingsKey]);
  const [sending, setSending] = useState(false);
  const [failed, setFailed] = useState(false);
  const inFlight = useRef(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const federated = federationEnabled && replyTo?.apub !== false && federatePreference;
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
    if (
      channelSelectionPending ||
      inFlight.current ||
      uploadLock.current ||
      (!content.trim() && !images.length) ||
      length > 1000
    )
      return;
    inFlight.current = true;
    setSending(true);
    setFailed(false);
    try {
      const settings: HomePostSettings = {
        apub: federated,
        visibility,
        channel_id: selectedChannelId || null,
      };
      await onPost({
        content,
        ...(images.length ? { image_ids: images.map((image) => image.id!) } : {}),
        ...settings,
        ...(replyTo ? { in_reply_to: replyTo.id, channel_id: null } : {}),
      });
      saveHomePostSettings(settingsKey, settings);
      setFederatePreference(settings.apub);
      setChannelId(settings.channel_id ?? "");
      setReplyTo(null);
      setContent("");
      images.forEach((image) => URL.revokeObjectURL(image.preview));
      setImages([]);
      imagesRef.current = [];
      setImageError(false);
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
          setVisibility((current) => replyVisibility(current, post.visibility, !!post.channel_id));
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
      {open ? (
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
                            (!!replyTo?.channel_id && value === "followers") ||
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
                  disabled={
                    sending ||
                    uploading ||
                    channelSelectionPending ||
                    (!content.trim() && !images.length) ||
                    length > 1000
                  }
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
                    <div className="mfm-body mt-1 line-clamp-2 break-words text-muted-foreground">
                      <ActivityPubContent text={replyTo.content} emojis={replyTo.emojis} />
                    </div>
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
                onPaste={(event) => {
                  const files = attachmentFilesFromClipboard(event.clipboardData).filter((file) =>
                    resolveAttachmentContentType(file)?.startsWith("image/"),
                  );
                  if (files.length) {
                    event.preventDefault();
                    void addImages(files);
                  }
                }}
                disabled={sending}
                aria-label={t("home.compose")}
                aria-describedby="home-post-count"
                aria-invalid={length > 1000 || undefined}
                placeholder={t("home.placeholder")}
                className="min-h-40 flex-1 resize-none border-0 p-0 shadow-none focus-visible:ring-0 md:min-h-48"
              />
              <input
                ref={fileInput}
                type="file"
                accept="image/png,image/jpeg,image/gif,image/webp"
                multiple
                hidden
                onChange={(event) => {
                  void addImages(Array.from(event.target.files ?? []));
                  event.target.value = "";
                }}
              />
              <Button
                type="button"
                variant="ghost"
                disabled={sending || uploading || images.length >= 10}
                onClick={() => fileInput.current?.click()}
                aria-label={t("attachments.add")}
              >
                <ImagePlusIcon aria-hidden="true" />
                {t("attachments.add")}
              </Button>
              {images.length ? (
                <ul aria-label={t("attachments.selected")} className="flex flex-wrap gap-2">
                  {images.map((image) => (
                    <li key={image.preview} className="relative">
                      <img
                        src={image.preview}
                        alt={image.file.name}
                        className="size-24 rounded object-cover"
                      />
                      <Button
                        type="button"
                        variant="secondary"
                        size="icon-sm"
                        className="absolute right-0 top-0"
                        disabled={sending || uploading}
                        aria-label={t("attachments.remove", { name: image.file.name })}
                        onClick={() => {
                          URL.revokeObjectURL(image.preview);
                          if (image.id) discardImage(image.id);
                          setImages((current) => current.filter((item) => item !== image));
                        }}
                      >
                        <XIcon aria-hidden="true" />
                      </Button>
                    </li>
                  ))}
                </ul>
              ) : null}
              {uploading ? <p role="status">{t("attachments.uploading")}</p> : null}
              {imageError ? (
                <p role="alert" className="text-sm text-destructive">
                  {t("home.imageError")}
                </p>
              ) : null}
              <div className="flex flex-wrap items-center justify-between gap-2">
                <label className="flex min-w-0 items-center gap-2 text-xs">
                  {t("home.channel")}
                  <Select
                    aria-label={t("home.channel")}
                    value={selectedChannelId}
                    disabled={sending || channelSelectionPending || visibility === "followers"}
                    onChange={(e) => setChannelId(e.target.value)}
                  >
                    <option value="">{t("home.noChannel")}</option>
                    {availableChannels.map((channel) => (
                      <option key={channel.id} value={channel.id}>
                        #{channel.channel_name}
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
      ) : null}
    </Dialog>
  );
}
