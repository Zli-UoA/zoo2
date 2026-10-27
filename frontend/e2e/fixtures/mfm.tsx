import { MessageBody } from "../../src/components/chat/messages/MessageRow";
import { Composer } from "../../src/components/chat/messages/Composer";
import { LocaleProvider } from "../../src/components/i18n/LocaleProvider";
import { createRoot } from "react-dom/client";
import { MfmChannelsProvider } from "../../src/components/chat/messages/MfmChannels";
import { MfmContent } from "../../src/components/chat/messages/MfmContent";
import "../../src/styles.css";

createRoot(document.getElementById("app")!).render(
  <MfmChannelsProvider
    channels={[
      { id: "dev-id", channel_name: "開発", deleted_at: null },
      { id: "old-id", channel_name: "削除済み", deleted_at: "2026-10-25" },
    ]}
  >
    <main style={{ padding: 24 }}>
      <LocaleProvider>
        <Composer
          mentionUsers={[
            {
              id: "taro",
              user_name: "taro",
              display_name: "太郎",
              created_at: "2026-10-27T00:00:00Z",
              updated_at: null,
            },
          ]}
          mentionMemberIds={new Set(["taro"])}
          onSend={async (content) => {
            document.getElementById("sent")!.textContent = content;
            return true;
          }}
        />
      </LocaleProvider>
      <output id="sent" />
      <section id="containment" style={{ width: 240, background: "white", color: "black" }}>
        <button
          id="previous-row"
          style={{ display: "block", width: "100%", height: 32 }}
          onClick={(event) => {
            event.currentTarget.textContent = "clicked previous";
          }}
        >
          previous row
        </button>
        <div id="transformed-row">
          <MessageBody content="$[position.y=-50 $[clickable overlay up]] $[position.y=50 $[clickable overlay down]] $[scale.x=5,y=5 $[scale.x=5,y=5 $[scale.x=5,y=5 $[clickable ■■■■■■■■]]]] $[rotate.deg=90 $[clickable rotated overlay]]" />
        </div>
        <button
          id="next-row"
          style={{ display: "block", width: "100%", height: 32 }}
          onClick={(event) => {
            event.currentTarget.textContent = "clicked next";
          }}
        >
          next row
        </button>
      </section>
      <div id="winter-time">
        <MfmContent content="$[unixtime 0]" />
      </div>
      <div id="summer-time">
        <MfmContent content="$[unixtime 1593561600]" />
      </div>
      <div id="hashtags">
        <MfmContent content="#開発 #未登録 #削除済み" />
      </div>
      <div id="animations">
        <MfmContent content="$[spin.x,left,speed=2s,delay=-1s 🌟] $[twitch 🐱] $[sparkle hello] $[rainbow rainbow]" />
      </div>
      <div id="blur">
        <MfmContent content="$[blur hidden]" />
      </div>
      <div id="click">
        <MfmContent
          content="$[clickable.ev=test click me]"
          onClickEvent={(event) => {
            document.getElementById("result")!.textContent = event;
          }}
        />
      </div>
      <output id="result" />
      <div id="emoji">
        <MfmContent content=":party: :smile:" />
      </div>
      <div id="math">
        <MfmContent content={"\\(\\frac{1}{2}\\)"} />
      </div>
      <div id="format">
        <MfmContent content="$[ruby **漢字** かんじ] $[border.width=2,color=f00,style=dashed $[fg.color=00f blue]]" />
      </div>
    </main>
  </MfmChannelsProvider>,
);
