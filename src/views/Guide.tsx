import {
  ArrowRight,
  Cat,
  Check,
  Eye,
  Lightbulb,
  ListVideo,
  ShieldCheck,
  Workflow,
  type LucideIcon,
} from "lucide-react";

import { useStaggerIn } from "@/lib/anime";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import type { ViewProps } from "./types";

export function Guide({ navigate }: ViewProps) {
  const pageRef = useStaggerIn<HTMLDivElement>();

  return (
    <div ref={pageRef} className="space-y-8">
      <header className="space-y-3">
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">Guide</h1>
        <p className="max-w-2xl text-sm leading-relaxed text-muted-foreground">
          Clawmation records what you do — every click, keypress and pause — and plays it back for
          you, as many times as you like. It can also watch your screen and step in on its own while
          you’re away, so your grind keeps running even when you’re not at the keyboard.
        </p>
      </header>

      {/* Record */}
      <Section icon={ListVideo} title="Record your first macro">
        <p className="text-sm leading-relaxed text-muted-foreground">
          A macro is just a recording of you playing. Do the task once, and Clawmation can repeat it
          for hours.
        </p>
        <ol className="flex flex-col gap-2.5 text-sm text-muted-foreground">
          <NumStep n={1}>
            Open your game, then press your record hotkey — or hit the Record button on the Macros
            page.
          </NumStep>
          <NumStep n={2}>
            Play through the task once, exactly how you normally would. Every click, key and pause is
            captured.
          </NumStep>
          <NumStep n={3}>Press Stop to save it, and give it a name you’ll recognise later.</NumStep>
          <NumStep n={4}>
            Press Run and choose how many times to repeat — a set number, or ∞ to keep going.
          </NumStep>
        </ol>
        <Note icon={Cat}>
          While a macro is running, a little cat sits in the corner — wide awake when Clawmation is
          playing, curled up asleep when it’s finished — so you can tell at a glance whether it’s
          still going.
        </Note>
        <Cta onClick={() => navigate("macros")}>Record a macro</Cta>
      </Section>

      {/* Guards */}
      <Section icon={ShieldCheck} title="Keep it alive with guards" badge="Most loved">
        <p className="text-sm leading-relaxed text-muted-foreground">
          Left alone, a farming loop dies the moment the game boots you to a login screen. A guard is
          a little watcher that keeps it alive. The classic case: your loop is set to run forever, a
          guard watches for the <span className="font-medium text-foreground">Reconnect</span> button,
          and the instant it appears the guard pauses your macro, clicks Reconnect, waits for the game
          to load back in, then carries on right where it left off — so the loop never dies.
        </p>
        <ol className="flex flex-col gap-2.5 text-sm text-muted-foreground">
          <NumStep n={1}>
            Tell it what to look for and where — a colour, a button or some words, in one corner of the
            screen.
          </NumStep>
          <NumStep n={2}>
            It checks that spot a few times every second, quietly in the background while your macro
            runs.
          </NumStep>
          <NumStep n={3}>
            The moment it spots the trigger, it pauses your macro and does what you set — a click or a
            keypress.
          </NumStep>
          <NumStep n={4}>
            After a short wait for the game to catch up, it picks your macro back up exactly where it
            paused.
          </NumStep>
        </ol>
        <div className="rounded-lg border border-border bg-secondary/40 p-4">
          <p className="mb-3 text-xs font-medium uppercase tracking-wider text-muted-foreground">
            The classic reconnect setup
          </p>
          <dl className="space-y-2 text-sm">
            <SpecRow label="Macro">your farming loop, set to ∞ reps</SpecRow>
            <SpecRow label="Trigger">the colour of the Reconnect button</SpecRow>
            <SpecRow label="Action">click it</SpecRow>
            <SpecRow label="Wait after">about 10 seconds</SpecRow>
          </dl>
        </div>
        <p className="text-sm text-muted-foreground">
          Guards live on each macro. Open a macro’s menu on the Macros page and add a guard to it.
        </p>
        <div className="flex flex-wrap gap-2">
          <Cta onClick={() => navigate("ai")}>See what’s protected</Cta>
          <Button variant="outline" onClick={() => navigate("macros")}>
            Go to Macros
          </Button>
        </div>
      </Section>

      {/* Watch */}
      <Section icon={Eye} title="Let it watch the screen on its own">
        <p className="text-sm leading-relaxed text-muted-foreground">
          Sometimes you don’t need a whole macro — you just need Clawmation to catch one thing and
          react. That’s the Watch page.
        </p>
        <ol className="flex flex-col gap-2.5 text-sm text-muted-foreground">
          <NumStep n={1}>
            Add a thing to watch for — a colour, a picture of a button, or some on-screen text.
          </NumStep>
          <NumStep n={2}>Tell it what to do when that thing appears — click it, or press a key.</NumStep>
          <NumStep n={3}>Press Start watching, and Clawmation reacts on its own. No macro required.</NumStep>
        </ol>
        <p className="text-sm text-muted-foreground">
          Handy for catching AFK check popups, auto-accepting invites, or clicking through a reward
          screen while you’re away.
        </p>
        <Cta onClick={() => navigate("vision")}>Open Watch</Cta>
      </Section>

      {/* Chains */}
      <Section icon={Workflow} title="Chain macros together">
        <p className="text-sm leading-relaxed text-muted-foreground">
          Have a few macros that always run in the same order? Chain them so they play back-to-back —
          optionally on a schedule, so your daily routine can even start itself.
        </p>
        <Cta onClick={() => navigate("chains")}>Build a chain</Cta>
      </Section>

      {/* Tips */}
      <Section icon={Lightbulb} title="A few tips">
        <ul className="flex flex-col gap-2.5 text-sm text-muted-foreground">
          <Tip>
            Keep the area you watch small — it’s faster and far less likely to fire on the wrong
            thing.
          </Tip>
          <Tip>
            Always Test a guard or trigger before you rely on it, so you know it can really see what
            you meant.
          </Tip>
          <Tip>
            Give macros clear names and categories — future-you will thank you once you have twenty of
            them.
          </Tip>
        </ul>
      </Section>
    </div>
  );
}

function Section({
  icon: Icon,
  title,
  badge,
  children,
}: {
  icon: LucideIcon;
  title: string;
  badge?: string;
  children: React.ReactNode;
}) {
  return (
    <Card className="gap-5 p-6">
      <div className="flex items-center gap-3">
        <div className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-primary/15 text-primary">
          <Icon className="size-5" />
        </div>
        <h2 className="text-lg font-semibold text-foreground">{title}</h2>
        {badge && (
          <Badge variant="secondary" className="ml-auto font-normal">
            {badge}
          </Badge>
        )}
      </div>
      {children}
    </Card>
  );
}

function NumStep({ n, children }: { n: number; children: React.ReactNode }) {
  return (
    <li className="flex gap-3">
      <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-primary/15 text-xs font-semibold text-primary">
        {n}
      </span>
      {children}
    </li>
  );
}

function SpecRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex gap-3">
      <dt className="w-24 shrink-0 text-muted-foreground">{label}</dt>
      <dd className="text-foreground">{children}</dd>
    </div>
  );
}

function Note({ icon: Icon, children }: { icon: LucideIcon; children: React.ReactNode }) {
  return (
    <div className="flex items-start gap-3 rounded-lg border border-border bg-secondary/40 p-3.5 text-sm text-muted-foreground">
      <Icon className="mt-0.5 size-4 shrink-0 text-primary" />
      <p className="leading-relaxed">{children}</p>
    </div>
  );
}

function Tip({ children }: { children: React.ReactNode }) {
  return (
    <li className="flex gap-3">
      <Check className="mt-0.5 size-4 shrink-0 text-primary" />
      {children}
    </li>
  );
}

function Cta({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <Button onClick={onClick} className="w-fit">
      {children}
      <ArrowRight className="size-4" />
    </Button>
  );
}
