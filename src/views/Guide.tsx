import {
  ArrowRight,
  CursorClick,
  Eye,
  GitBranch,
  Keyboard,
  LinkSimple,
  Play,
  Record,
  ShieldCheck,
  WarningCircle,
  type Icon,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import type { ViewProps } from "./types";

export type GuideTopic =
  | "getting-started"
  | "macros"
  | "loops"
  | "watch"
  | "troubleshooting";

export interface GuideTopicMeta {
  id: GuideTopic;
  label: string;
  description: string;
  Icon: Icon;
}

export const GUIDE_TOPICS: GuideTopicMeta[] = [
  {
    id: "getting-started",
    label: "Getting started",
    description: "Record, run, and stop your first automation.",
    Icon: Play,
  },
  {
    id: "macros",
    label: "Macros",
    description: "Recording, playback, repeats, and safeguards.",
    Icon: Record,
  },
  {
    id: "loops",
    label: "Loops & chains",
    description: "Build visual workflows and ordered macro sequences.",
    Icon: GitBranch,
  },
  {
    id: "watch",
    label: "Watch & vision",
    description: "Detect images, colours, and screen states reliably.",
    Icon: Eye,
  },
  {
    id: "troubleshooting",
    label: "Troubleshooting",
    description: "Fast answers for playback, hotkeys, and vision.",
    Icon: WarningCircle,
  },
];

interface GuideProps extends Pick<ViewProps, "navigate"> {
  topic: GuideTopic;
}

export function Guide({ topic, navigate }: GuideProps) {
  switch (topic) {
    case "macros":
      return <MacrosGuide navigate={navigate} />;
    case "loops":
      return <LoopsGuide navigate={navigate} />;
    case "watch":
      return <WatchGuide navigate={navigate} />;
    case "troubleshooting":
      return <TroubleshootingGuide navigate={navigate} />;
    default:
      return <GettingStartedGuide navigate={navigate} />;
  }
}

function GettingStartedGuide({
  navigate,
}: Pick<ViewProps, "navigate">) {
  return (
    <Article
      eyebrow="Documentation"
      title="Getting started"
      intro="Create a recording, play it back, and stay in control from the first run."
    >
      <DocSection title="How Clawmation works">
        <p>
          A macro captures your mouse, keyboard, timing, and camera drags. Playing
          it repeats the same actions against the same screen layout.
        </p>
        <DefinitionList
          items={[
            ["Macro", "A recording of one task."],
            ["Watch", "A screen detector that reacts when something appears."],
            ["Loop", "A visual workflow connecting macros, waits, decisions, and chains."],
          ]}
        />
      </DocSection>

      <DocSection title="Record your first macro">
        <Steps
          items={[
            "Open the game or app at the exact size and position you plan to use.",
            "Press Record from Macros, then perform the task once at a natural pace.",
            "Use the Stop hotkey when the task is complete. Clawmation saves it immediately.",
            "Rename the recording and choose its repeat count before the first run.",
          ]}
        />
        <Callout icon={Record}>
          Right-click camera movement, Shift Lock, held keys, and pauses are all
          part of the recording. Perform them exactly as you want them replayed.
        </Callout>
      </DocSection>

      <DocSection title="Run safely">
        <p>
          Start with one repeat while watching the game. If the route is correct,
          increase the repeat count or choose infinity.
        </p>
        <ShortcutRow keys={["Stop hotkey"]}>
          Stops macros, chains, Loops, Watch actions, and recording from one place.
        </ShortcutRow>
        <ArticleAction onClick={() => navigate("macros")}>
          Open Macros
        </ArticleAction>
      </DocSection>
    </Article>
  );
}

function MacrosGuide({ navigate }: Pick<ViewProps, "navigate">) {
  return (
    <Article
      eyebrow="Documentation"
      title="Macros"
      intro="Record dependable actions, organize them, and recover when the screen changes."
    >
      <DocSection title="The macro workspace">
        <p>
          Select a macro to edit its name, category, notes, speed, and repeat
          behavior. The library stays independently scrollable so controls never
          disappear while you browse.
        </p>
        <FeatureList
          items={[
            ["Playback speed", "Use 1× for recorded timing. Faster speeds reduce every recorded delay."],
            ["Repeat", "Run once, a fixed number of times, or continuously until stopped."],
            ["Presets", "Save a setup you want to reuse without duplicating the source file manually."],
            ["Bundles", "Export a macro together with the images its vision steps need."],
          ]}
        />
      </DocSection>

      <DocSection title="Screen safeguards">
        <p>
          Safety and Vision live beside the selected macro. They let a long run
          recover from disconnects, wait for loading screens, or click a known
          button before continuing.
        </p>
        <Steps
          items={[
            "Choose the smallest reliable image, colour, or screen region.",
            "Test the detector against a fresh frame before enabling it.",
            "Set the action and a realistic timeout for the game to respond.",
            "Run the macro once while watching both success and failure paths.",
          ]}
        />
        <Callout icon={ShieldCheck}>
          A smaller detection region is faster and less likely to match the wrong
          part of the screen.
        </Callout>
      </DocSection>

      <DocSection title="Keep recordings reliable">
        <FeatureList
          items={[
            ["Window geometry", "Keep the game at the same size and display scale used while recording."],
            ["Camera control", "Record deliberate right-button drags and avoid moving the physical mouse during playback."],
            ["Timing", "Leave enough time for menus and teleports to finish before the next action."],
            ["Emergency stop", "Set a memorable Stop hotkey and test it before running continuously."],
          ]}
        />
        <ArticleAction onClick={() => navigate("macros")}>
          Manage macros
        </ArticleAction>
      </DocSection>
    </Article>
  );
}

function LoopsGuide({ navigate }: Pick<ViewProps, "navigate">) {
  return (
    <Article
      eyebrow="Documentation"
      title="Loops & chains"
      intro="Turn individual recordings into readable workflows with clear success and failure paths."
    >
      <DocSection title="Build on the canvas">
        <Steps
          items={[
            "Create a Loop and right-click anywhere on the canvas.",
            "Add a Start node, then add macros, waits, vision checks, or actions.",
            "Drag from an output handle to the next node. Each output accepts one destination.",
            "Connect every required path, then Save and Run from the toolbar.",
          ]}
        />
        <ShortcutGrid
          items={[
            ["Ctrl + Z", "Undo"],
            ["Ctrl + Shift + Z", "Redo"],
            ["Ctrl + S", "Save Loop"],
            ["Ctrl + D", "Duplicate node"],
            ["Delete", "Remove selected node"],
            ["F2", "Rename Loop"],
          ]}
        />
      </DocSection>

      <DocSection title="Read outcome paths">
        <p>
          Nodes that can fail expose two named outputs. Connect both whenever the
          failure needs recovery instead of stopping the workflow.
        </p>
        <div className="grid gap-3 sm:grid-cols-2">
          <Outcome tone="success" title="If works">
            Continue to the normal next step.
          </Outcome>
          <Outcome tone="danger" title="If fails">
            Retry, recover, notify, or stop safely.
          </Outcome>
        </div>
      </DocSection>

      <DocSection title="Compose a chain inside a Loop">
        <p>
          A Chain node runs several saved macros in order while keeping the
          canvas compact.
        </p>
        <Steps
          items={[
            "Add a Chain node and create a saved chain from its inspector.",
            "Add macros to the sequence. Drag rows or use the arrow controls to reorder them.",
            "Choose the pause between macros and how many times the whole sequence repeats.",
            "Save the chain, connect If works and If fails, then save the Loop.",
          ]}
        />
        <Callout icon={LinkSimple}>
          Use connected Macro nodes when each step needs its own branch. Use a
          Chain node when the sequence always runs straight through.
        </Callout>
        <ArticleAction onClick={() => navigate("nodes")}>
          Open Loops
        </ArticleAction>
      </DocSection>
    </Article>
  );
}

function WatchGuide({ navigate }: Pick<ViewProps, "navigate">) {
  return (
    <Article
      eyebrow="Documentation"
      title="Watch & vision"
      intro="Detect the right screen state quickly, then perform a click or key action."
    >
      <DocSection title="Choose the detector">
        <DefinitionList
          items={[
            ["Image", "Best for distinctive buttons, icons, and objects that keep the same appearance."],
            ["Colour", "Fastest for a stable, unique colour inside a small region."],
            ["Text", "Useful when the wording is stable but surrounding visuals change."],
          ]}
        />
      </DocSection>

      <DocSection title="Capture a clean image">
        <Steps
          items={[
            "Capture only the visual detail that identifies the target.",
            "Avoid animated edges, counters, player names, and changing backgrounds.",
            "Restrict the search region to where the target can realistically appear.",
            "Test several times before enabling Press or Click.",
          ]}
        />
        <Callout icon={CursorClick}>
          A detector finding the object does not automatically click it. Confirm
          the action is set to Click or Key instead of None.
        </Callout>
      </DocSection>

      <DocSection title="Tune reliability">
        <FeatureList
          items={[
            ["Confidence", "Raise it to reject false matches; lower it slightly if a correct image is missed."],
            ["Region", "The strongest speed and accuracy improvement. Keep it as small as practical."],
            ["Timeout", "Give loading screens enough time, but always choose what happens when time expires."],
            ["Test frame", "Use a fresh capture after changing game resolution, UI scale, or theme."],
          ]}
        />
        <ArticleAction onClick={() => navigate("vision")}>
          Open Watch
        </ArticleAction>
      </DocSection>
    </Article>
  );
}

function TroubleshootingGuide({
  navigate,
}: Pick<ViewProps, "navigate">) {
  return (
    <Article
      eyebrow="Documentation"
      title="Troubleshooting"
      intro="Start with the symptom, verify the smallest possible case, then change one variable."
    >
      <DocSection title="A run will not stop">
        <Steps
          items={[
            "Press the configured Stop hotkey once.",
            "If the app is visible, press Stop in the top bar.",
            "Open Settings → Shortcuts and confirm the key is registered and not shared with another action.",
            "Test the Stop hotkey with a one-step macro before starting a long run.",
          ]}
        />
      </DocSection>

      <DocSection title="Clicks or camera movement land incorrectly">
        <FeatureList
          items={[
            ["Display scale", "Use the same Windows scale and monitor arrangement as the recording."],
            ["Window position", "Keep the game window at the recorded size and location."],
            ["Input ownership", "Do not move the physical mouse while playback owns a held drag."],
            ["Game state", "Confirm menus, Shift Lock, and camera mode match the beginning of the recording."],
          ]}
        />
      </DocSection>

      <DocSection title="Vision misses the target">
        <Steps
          items={[
            "Capture a tighter image with fewer changing pixels.",
            "Select a smaller region and test against a new frame.",
            "Lower confidence in small steps instead of making a large jump.",
            "Re-capture after any resolution, UI scale, or graphics change.",
          ]}
        />
      </DocSection>

      <DocSection title="Hotkeys or settings do not persist">
        <p>
          Use a unique shortcut for each action. If Windows refuses a key,
          another app has registered it globally. Choose a different combination,
          then verify it immediately without restarting.
        </p>
        <ArticleAction onClick={() => navigate("settings")}>
          Open Settings
        </ArticleAction>
      </DocSection>
    </Article>
  );
}

function Article({
  eyebrow,
  title,
  intro,
  children,
}: {
  eyebrow: string;
  title: string;
  intro: string;
  children: React.ReactNode;
}) {
  return (
    <article className="mx-auto w-full max-w-3xl pb-16">
      <header className="border-b border-border pb-8">
        <p className="text-xs font-medium text-primary">{eyebrow}</p>
        <h1
          id="guide-article-title"
          className="mt-2 text-[2rem] font-semibold tracking-[-0.04em] text-foreground"
        >
          {title}
        </h1>
        <p className="mt-3 max-w-2xl text-[15px] leading-7 text-muted-foreground">
          {intro}
        </p>
      </header>
      <div className="divide-y divide-border">{children}</div>
    </article>
  );
}

function DocSection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-5 py-8 text-sm leading-7 text-muted-foreground">
      <h2 className="text-lg font-semibold tracking-[-0.02em] text-foreground">
        {title}
      </h2>
      {children}
    </section>
  );
}

function Steps({ items }: { items: string[] }) {
  return (
    <ol className="space-y-4">
      {items.map((item, index) => (
        <li key={item} className="grid grid-cols-[1.75rem_minmax(0,1fr)] gap-3">
          <span className="grid size-7 place-items-center rounded-md bg-primary/10 text-xs font-semibold tabular-nums text-primary">
            {index + 1}
          </span>
          <p className="pt-px">{item}</p>
        </li>
      ))}
    </ol>
  );
}

function FeatureList({ items }: { items: [string, string][] }) {
  return (
    <dl className="divide-y divide-border border-y border-border">
      {items.map(([term, detail]) => (
        <div
          key={term}
          className="grid gap-1 py-3.5 sm:grid-cols-[9rem_minmax(0,1fr)] sm:gap-5"
        >
          <dt className="font-medium text-foreground">{term}</dt>
          <dd>{detail}</dd>
        </div>
      ))}
    </dl>
  );
}

function DefinitionList({ items }: { items: [string, string][] }) {
  return <FeatureList items={items} />;
}

function Callout({
  icon: IconComponent,
  children,
}: {
  icon: Icon;
  children: React.ReactNode;
}) {
  return (
    <div className="grid grid-cols-[auto_minmax(0,1fr)] gap-3 border-l-2 border-primary/55 bg-primary/[0.035] py-3 pr-4 pl-3.5">
      <IconComponent className="mt-1 size-4 text-primary" weight="duotone" />
      <p>{children}</p>
    </div>
  );
}

function ShortcutRow({
  keys,
  children,
}: {
  keys: string[];
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-start gap-4 border-y border-border py-4">
      <div className="flex shrink-0 items-center gap-1.5">
        <Keyboard className="size-4 text-primary" />
        {keys.map((key) => (
          <kbd
            key={key}
            className="rounded border border-border bg-muted/45 px-2 py-0.5 font-mono text-[10px] text-foreground"
          >
            {key}
          </kbd>
        ))}
      </div>
      <p>{children}</p>
    </div>
  );
}

function ShortcutGrid({ items }: { items: [string, string][] }) {
  return (
    <div className="grid border-y border-border sm:grid-cols-2">
      {items.map(([keys, action], index) => (
        <div
          key={keys}
          className={cnBorder(index)}
        >
          <kbd className="font-mono text-[11px] font-medium text-foreground">
            {keys}
          </kbd>
          <span>{action}</span>
        </div>
      ))}
    </div>
  );
}

function cnBorder(index: number) {
  return [
    "flex items-center justify-between gap-4 py-3 text-xs",
    index > 1 ? "border-t border-border" : "",
    index % 2 === 0 ? "sm:pr-4" : "sm:border-l sm:border-border sm:pl-4",
  ].join(" ");
}

function Outcome({
  tone,
  title,
  children,
}: {
  tone: "success" | "danger";
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-start gap-3 border-y border-border py-3.5">
      <span
        className={`mt-1.5 size-2 shrink-0 rounded-full ${
          tone === "success" ? "bg-success" : "bg-destructive"
        }`}
      />
      <div>
        <p
          className={`font-medium ${
            tone === "success" ? "text-success" : "text-destructive"
          }`}
        >
          {title}
        </p>
        <p>{children}</p>
      </div>
    </div>
  );
}

function ArticleAction({
  onClick,
  children,
}: {
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <Button onClick={onClick}>
      {children}
      <ArrowRight className="size-4" />
    </Button>
  );
}
