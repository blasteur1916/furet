// La coque de l'île : sa forme noire, animée par des ressorts vers la forme voulue.
import { useEffect, type ReactNode } from "react";
import { animate, motion, useMotionValue, useTransform, type MotionValue } from "motion/react";
import type { ShellTarget } from "./geometrie";
import { bodyPath, FLARE, leftFlarePath, rightFlarePath, silhouetteWidth } from "./shell";
import { DURATIONS, SPRINGS, type Spring } from "./springs";

interface IleShellProps {
  /** Où va la coque. La changer en plein vol réoriente les ressorts sans les relancer. */
  target: ShellTarget;
  /** Forme au montage, avant de tendre vers `target`. */
  initial: ShellTarget;
  reducedMotion: boolean;
  children?: ReactNode;
}

/**
 * La surface physique de l'île. Chaque dimension est une valeur animée par un ressort : une nouvelle
 * cible ne relance jamais l'animation, elle la courbe en gardant sa vitesse. Le contenu vit dans une
 * découpe qui suit les mêmes valeurs.
 */
export function IleShell({ target, initial, reducedMotion, children }: IleShellProps) {
  const width = useMotionValue(initial.width);
  const height = useMotionValue(initial.height);
  const topRadius = useMotionValue(initial.topRadius);
  const bottomRadius = useMotionValue(initial.bottomRadius);
  const flares = useMotionValue(initial.flares);
  const fill = useMotionValue(initial.fill);
  const y = useMotionValue(initial.y);

  const { width: tw, height: th, topRadius: ttr, bottomRadius: tbr, flares: tf, fill: tfill, y: ty } = target;
  useEffect(() => {
    const spring = (s: Spring) => (reducedMotion ? { duration: 0.12 } : s);
    const tween = (seconds: number) => ({ duration: seconds, ease: "easeOut" as const });
    // Grandir utilise le ressort le plus vif, rétrécir le plus calme, axe par axe.
    const pick = (value: MotionValue<number>, to: number, grow: Spring, shrink: Spring) =>
      spring(to >= value.get() ? grow : shrink);

    animate(width, tw, pick(width, tw, SPRINGS.widthOpen, SPRINGS.widthClose));
    animate(height, th, pick(height, th, SPRINGS.heightOpen, SPRINGS.heightClose));
    animate(topRadius, ttr, spring(SPRINGS.radius));
    animate(bottomRadius, tbr, spring(SPRINGS.radius));
    animate(y, ty, spring(SPRINGS.y));
    animate(flares, tf, tween(DURATIONS.flares));
    animate(fill, tfill, tween(DURATIONS.fill));
  }, [tw, th, ttr, tbr, tf, tfill, ty, reducedMotion, width, height, topRadius, bottomRadius, flares, fill, y]);

  const total = useTransform(width, silhouetteWidth);
  const body = useTransform([width, height, topRadius, bottomRadius], ([w, h, t, b]: number[]) =>
    bodyPath({ width: w, height: h, topRadius: t, bottomRadius: b }),
  );
  const rightFlare = useTransform(width, rightFlarePath);

  return (
    <motion.div className="ile" style={{ width: total, height, y }}>
      <svg className="ile-svg" aria-hidden="true">
        <motion.g style={{ opacity: fill }}>
          <motion.path d={body} />
          <motion.path d={leftFlarePath()} style={{ opacity: flares }} />
          <motion.path d={rightFlare} style={{ opacity: flares }} />
        </motion.g>
      </svg>
      <motion.div
        className="ile-clip"
        // La découpe ne doit jamais défiler (un focus, une sélection au clavier pourraient la faire bouger).
        onScroll={(e) => {
          e.currentTarget.scrollTop = 0;
          e.currentTarget.scrollLeft = 0;
        }}
        style={{
          left: FLARE,
          width,
          height,
          borderTopLeftRadius: topRadius,
          borderTopRightRadius: topRadius,
          borderBottomLeftRadius: bottomRadius,
          borderBottomRightRadius: bottomRadius,
        }}
      >
        {children}
      </motion.div>
    </motion.div>
  );
}
