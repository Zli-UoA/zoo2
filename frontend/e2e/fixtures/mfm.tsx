import { createRoot } from "react-dom/client";
import { MfmContent } from "../../src/components/chat/messages/MfmContent";
import "../../src/styles.css";

createRoot(document.getElementById("app")!).render(
  <main style={{ padding: 24 }}>
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
  </main>,
);
