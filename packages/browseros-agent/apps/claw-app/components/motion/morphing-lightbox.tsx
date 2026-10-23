"use client";
// beui.dev/components/motion/image-viewer

import { ChevronLeft, ChevronRight, Minus, Plus, X } from "lucide-react";
import {
  type HTMLMotionProps,
  type Variants,
  AnimatePresence,
  LayoutGroup,
  motion,
  useMotionValue,
  useIsPresent,
  useReducedMotion,
} from "motion/react";
import {
  type ComponentPropsWithRef,
  type ReactNode,
  createContext,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import {
  EASE_DRAWER,
  EASE_OUT,
  SPRING_LAYOUT,
  SPRING_PRESS,
} from "@/lib/ease";
import { useModalScope } from "@/lib/hooks/use-modal-scope";
import { PresenceGate } from "@/lib/presence-gate";
import { cn } from "@/lib/utils";

export interface LightboxImage {
  id: string;
  src: string;
  alt: string;
  /** Intrinsic dimensions preserve aspect ratio through the thumbnail morph. */
  width: number;
  height: number;
  caption?: string;
}

export interface MorphingLightboxProps {
  images: LightboxImage[];
  value?: string | null;
  defaultValue?: string | null;
  onValueChange?: (id: string | null) => void;
  label?: string;
  className?: string;
  thumbnailClassName?: string;
  renderCaption?: (image: LightboxImage) => ReactNode;
  /** Compose gallery and viewer parts instead of the default layout. */
  children?: ReactNode;
}

type ImageSwap = { direction: -1 | 1; reduce: boolean };
// Paging needs a definite arrival; an overdamped panel spring creeps through
// the last few pixels before snapping to its rest threshold.
const IMAGE_SLIDE_TRANSITION = {
  type: "tween",
  duration: 0.28,
  ease: EASE_DRAWER,
} as const;
// Use Motion's x channel so the final slide position is painted in the same
// frame as its animation. A native transform animation can briefly expose its
// initial inline transform when it releases ownership at completion.
const imageSwapVariants: Variants = {
  enter: ({ direction, reduce }: ImageSwap) => ({
    opacity: reduce ? 0 : 1,
    x: `${reduce ? 0 : direction * 100}%`,
  }),
  visible: ({ reduce }: ImageSwap) => ({
    opacity: 1,
    x: "0%",
    transition: reduce
      ? { duration: 0.1, ease: EASE_OUT }
      : IMAGE_SLIDE_TRANSITION,
  }),
  exit: ({ direction, reduce }: ImageSwap) => ({
    opacity: reduce ? 0 : 1,
    x: `${reduce ? 0 : direction * -100}%`,
    transition: reduce
      ? { duration: 0.1, ease: EASE_OUT }
      : IMAGE_SLIDE_TRANSITION,
  }),
};

const controlClass =
  "inline-flex size-10 shrink-0 items-center justify-center rounded-full bg-white/10 text-white transition-colors hover:bg-white/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white disabled:opacity-30";

export type ImageViewerImage = LightboxImage;
export type ImageViewerProps = MorphingLightboxProps;

type ViewerContextValue = {
  images: LightboxImage[];
  value: string | null;
  image: LightboxImage | undefined;
  index: number;
  label: string;
  groupId: string;
  reduce: boolean;
  renderCaption?: (image: LightboxImage) => ReactNode;
  select: (id: string | null) => void;
  move: (direction: -1 | 1) => void;
};
const ViewerContext = createContext<ViewerContextValue | null>(null);

function useViewerContext(part: string) {
  const context = useContext(ViewerContext);
  if (!context) throw new Error(`${part} must be used within ImageViewer.`);
  return context;
}

/** Shared selection and navigation for custom gallery and viewer controls. */
export function useImageViewer() {
  const { images, value, image, index, select, move } =
    useViewerContext("useImageViewer");
  return {
    images,
    value,
    image,
    index,
    hasPrevious: index > 0,
    hasNext: index >= 0 && index < images.length - 1,
    select,
    close: () => select(null),
    previous: () => move(-1),
    next: () => move(1),
  };
}

/** A composable image gallery with a focus-managed, thumbnail-connected viewer. */
export function ImageViewer({
  images,
  value: controlledValue,
  defaultValue = null,
  onValueChange,
  label = "Image gallery",
  className,
  thumbnailClassName,
  renderCaption,
  children,
}: ImageViewerProps) {
  const [internalValue, setInternalValue] = useState(defaultValue);
  const value = controlledValue === undefined ? internalValue : controlledValue;
  const index = images.findIndex((image) => image.id === value);
  const image = images[index];
  const groupId = useId();
  const reduce = useReducedMotion() ?? false;
  if (new Set(images.map((item) => item.id)).size !== images.length)
    throw new Error("ImageViewer requires unique image ids.");
  if (
    images.some(
      (item) =>
        !Number.isFinite(item.width) ||
        !Number.isFinite(item.height) ||
        item.width <= 0 ||
        item.height <= 0,
    )
  )
    throw new Error("ImageViewer requires positive image dimensions.");
  const select = (id: string | null) => {
    if (controlledValue === undefined) setInternalValue(id);
    onValueChange?.(id);
  };
  // Drop removed identities instead of letting a re-added image reopen a session.
  if (controlledValue === undefined && internalValue !== null && index < 0)
    setInternalValue(null);
  const move = (direction: -1 | 1) => {
    const next = images[index + direction];
    if (image && next) select(next.id);
  };
  return (
    <ViewerContext.Provider
      value={{
        images,
        value,
        image,
        index,
        label,
        groupId,
        reduce,
        renderCaption,
        select,
        move,
      }}
    >
      <LayoutGroup id={groupId}>
        {children === undefined ? (
          <>
            <ImageViewerGallery className={className}>
              {images.map((item) => (
                <ImageViewerThumbnail
                  key={item.id}
                  imageId={item.id}
                  className={thumbnailClassName}
                />
              ))}
            </ImageViewerGallery>
            <ImageViewerContent />
          </>
        ) : (
          children
        )}
      </LayoutGroup>
    </ViewerContext.Provider>
  );
}

/** Generic name; the original export remains compatible with existing installs. */
export { ImageViewer as MorphingLightbox };

export function ImageViewerGallery({
  className,
  ...props
}: ComponentPropsWithRef<"section">) {
  const { label } = useViewerContext("ImageViewerGallery");
  return (
    <section
      aria-label={label}
      {...props}
      className={cn("grid w-full grid-cols-2 gap-3 sm:grid-cols-3", className)}
    />
  );
}

export interface ImageViewerThumbnailProps
  extends Omit<HTMLMotionProps<"button">, "children"> {
  imageId: string;
  imageClassName?: string;
  children?: ReactNode;
}

/** The image keeps its shared-layout identity when the trigger is customised. */
export function ImageViewerThumbnail({
  imageId,
  className,
  imageClassName,
  children,
  onClick,
  disabled,
  ...props
}: ImageViewerThumbnailProps) {
  const { images, value, groupId, reduce, select } = useViewerContext(
    "ImageViewerThumbnail",
  );
  const image = images.find((item) => item.id === imageId);
  if (!image)
    throw new Error(`ImageViewerThumbnail: unknown image id "${imageId}".`);
  return (
    <motion.button
      type="button"
      aria-label={`Open ${image.alt}`}
      whileTap={reduce ? undefined : { scale: 0.98 }}
      transition={SPRING_PRESS}
      {...props}
      disabled={disabled}
      aria-haspopup="dialog"
      aria-expanded={image.id === value}
      onClick={(event) => {
        onClick?.(event);
        if (!event.defaultPrevented && !disabled) select(image.id);
      }}
      className={cn(
        "relative overflow-hidden rounded-2xl bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
        className,
      )}
    >
      {/* biome-ignore lint/performance/noImgElement: Copy-paste registry images must work outside Next.js. */}
      <motion.img
        layoutId={reduce ? undefined : `${groupId}-${image.id}`}
        src={image.src}
        alt={image.alt}
        width={image.width}
        height={image.height}
        loading="lazy"
        draggable={false}
        transition={SPRING_LAYOUT}
        className={cn("block h-auto w-full", imageClassName)}
      />
      {children}
    </motion.button>
  );
}

export function ImageViewerCounter({
  className,
  ...props
}: ComponentPropsWithRef<"span">) {
  const { index, images } = useViewerContext("ImageViewerCounter");
  return (
    <span
      aria-live="polite"
      {...props}
      className={cn("text-xs tabular-nums", className)}
    >
      {index + 1} / {images.length}
    </span>
  );
}

export function ImageViewerCaption({
  className,
  children,
  ...props
}: ComponentPropsWithRef<"div">) {
  const { image, renderCaption } = useViewerContext("ImageViewerCaption");
  return (
    <div
      aria-live="polite"
      {...props}
      data-lightbox-content=""
      className={cn("min-w-0 text-center text-sm", className)}
    >
      {children ??
        (image &&
          (renderCaption
            ? renderCaption(image)
            : (image.caption ?? image.alt)))}
    </div>
  );
}

type ViewerButtonProps = ComponentPropsWithRef<"button">;
function ViewerButton({
  action,
  className,
  children,
  disabled,
  onClick,
  ...props
}: ViewerButtonProps & { action: "close" | "previous" | "next" }) {
  const { index, images, select, move } = useViewerContext(
    "ImageViewer controls",
  );
  const unavailable =
    action === "previous"
      ? index <= 0
      : action === "next"
        ? index < 0 || index >= images.length - 1
        : false;
  const Icon =
    action === "close" ? X : action === "previous" ? ChevronLeft : ChevronRight;
  return (
    <button
      type="button"
      aria-label={
        action === "close"
          ? "Close viewer"
          : action === "previous"
            ? "Previous image"
            : "Next image"
      }
      {...props}
      disabled={disabled || unavailable}
      onClick={(event) => {
        onClick?.(event);
        if (!event.defaultPrevented) {
          if (action === "close") select(null);
          else move(action === "previous" ? -1 : 1);
        }
      }}
      className={cn(controlClass, className)}
    >
      {children ?? <Icon size={18} aria-hidden="true" />}
    </button>
  );
}
export function ImageViewerClose(props: ViewerButtonProps) {
  return <ViewerButton {...props} action="close" />;
}
export function ImageViewerPrevious(props: ViewerButtonProps) {
  return <ViewerButton {...props} action="previous" />;
}
export function ImageViewerNext(props: ViewerButtonProps) {
  return <ViewerButton {...props} action="next" />;
}

export interface ImageViewerContentProps {
  className?: string;
  /** Custom header; defaults to the counter and close control. */
  header?: ReactNode;
  /** Custom footer; defaults to navigation and the image caption. */
  children?: ReactNode;
}

/** Owns the portal, focus scope, swipe/zoom frame and exit interaction gate. */
export function ImageViewerContent({
  className,
  header,
  children,
}: ImageViewerContentProps) {
  const context = useViewerContext("ImageViewerContent");
  const { image, index, label, groupId, reduce, select, move } = context;
  const [mounted, setMounted] = useState(false);
  const portal = useRef<HTMLDivElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    setMounted(true);
  }, []);
  useModalScope(mounted && Boolean(image), portal, panel, () => select(null));
  const selectedId = image?.id;
  const previousId = useRef<string | undefined>(undefined);
  useLayoutEffect(() => {
    const previous = previousId.current;
    previousId.current = mounted ? selectedId : undefined;
    if (!mounted || !selectedId || !previous || !panel.current) return;
    const focused = document.activeElement;
    if (
      !panel.current.contains(focused) ||
      (focused instanceof HTMLButtonElement && focused.disabled)
    ) {
      (
        panel.current.querySelector<HTMLButtonElement>(
          "button:not(:disabled)",
        ) ?? panel.current
      ).focus({ preventScroll: true });
    }
  }, [mounted, selectedId]);
  if (!mounted) return null;
  return createPortal(
    <div ref={portal}>
      <AnimatePresence>
        {image && (
          <PresenceGate>
            {({ isPresent, gate }) => (
              <ViewerContext.Provider value={context}>
                <motion.button
                  {...gate}
                  type="button"
                  tabIndex={-1}
                  aria-hidden="true"
                  onClick={() => select(null)}
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: reduce ? 0.1 : 0.18, ease: EASE_OUT }}
                  className="fixed inset-0 z-[100] h-full w-full cursor-default bg-black/90"
                />
                <div
                  inert={!isPresent}
                  className="pointer-events-none fixed inset-3 z-[101] sm:inset-6"
                >
                  <div
                    {...gate}
                    ref={panel}
                    role="dialog"
                    aria-modal="true"
                    aria-label={label}
                    tabIndex={-1}
                    className={cn(
                      "flex h-full flex-col outline-none",
                      className,
                    )}
                    onPointerDownCapture={(event) => {
                      const target = event.target;
                      if (
                        target instanceof Element &&
                        !target.closest(
                          "button, a, input, select, textarea, [data-lightbox-content]",
                        )
                      ) {
                        event.preventDefault();
                        select(null);
                      }
                    }}
                    onKeyDown={(event) => {
                      if (
                        (event.target instanceof Element &&
                          event.target.closest(
                            'input, textarea, select, [contenteditable="true"]',
                          )) ||
                        event.defaultPrevented ||
                        event.altKey ||
                        event.ctrlKey ||
                        event.metaKey ||
                        event.shiftKey
                      )
                        return;
                      if (
                        event.key === "ArrowLeft" ||
                        event.key === "ArrowRight"
                      ) {
                        event.preventDefault();
                        move(event.key === "ArrowLeft" ? -1 : 1);
                      }
                    }}
                  >
                    <div
                      data-lightbox-content=""
                      className="flex items-center justify-between gap-3 pb-3 text-white"
                    >
                      {header === undefined ? (
                        <>
                          <ImageViewerCounter />
                          <ImageViewerClose />
                        </>
                      ) : (
                        header
                      )}
                    </div>
                    <LightboxFrame
                      image={image}
                      index={index}
                      layoutId={reduce ? undefined : `${groupId}-${image.id}`}
                      reduce={reduce}
                      onSwipe={move}
                    />
                    <div
                      data-lightbox-content=""
                      className="flex items-center justify-between gap-3 pt-3 text-white"
                    >
                      {children === undefined ? (
                        <>
                          <ImageViewerPrevious />
                          <ImageViewerCaption />
                          <ImageViewerNext />
                        </>
                      ) : (
                        children
                      )}
                    </div>
                  </div>
                </div>
              </ViewerContext.Provider>
            )}
          </PresenceGate>
        )}
      </AnimatePresence>
    </div>,
    document.body,
  );
}

function LightboxFrame({
  image,
  index,
  layoutId,
  reduce,
  onSwipe,
}: {
  image: LightboxImage;
  index: number;
  layoutId?: string;
  reduce: boolean;
  onSwipe: (direction: -1 | 1) => void;
}) {
  const [zoom, setZoom] = useState(false);
  const [failed, setFailed] = useState(false);
  const session = `${image.id}-${image.src}`;
  const [selection, setSelection] = useState<{
    session: string;
    index: number;
    direction: -1 | 1;
  }>({ session, index, direction: 1 });
  const changed = selection.session !== session;
  const direction = changed
    ? index < selection.index
      ? -1
      : 1
    : selection.direction;
  // Keep the shared-layout frame mounted; only the image layer changes identity.
  if (changed) {
    setSelection({ session, index, direction });
    setZoom(false);
    setFailed(false);
  }
  const swipe = useRef<{ x: number; y: number } | null>(null);
  const present = useIsPresent();
  const bounds = useRef<HTMLDivElement>(null);
  const imageBox = useRef<HTMLDivElement>(null);
  const x = useMotionValue(0);
  const y = useMotionValue(0);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const panHelp = useId();
  const positionSession = useRef(session);
  useLayoutEffect(() => {
    if (positionSession.current === session) return;
    positionSession.current = session;
    x.set(0);
    y.set(0);
    swipe.current = null;
  }, [session, x, y]);
  useLayoutEffect(() => {
    const measure = () => {
      if (!bounds.current || !imageBox.current) return;
      const next = {
        x: Math.max(
          0,
          (imageBox.current.clientWidth * 2 - bounds.current.clientWidth) / 2,
        ),
        y: Math.max(
          0,
          (imageBox.current.clientHeight * 2 - bounds.current.clientHeight) / 2,
        ),
      };
      setPan(next);
      x.set(Math.max(-next.x, Math.min(next.x, x.get())));
      y.set(Math.max(-next.y, Math.min(next.y, y.get())));
    };
    measure();
    const observer = new ResizeObserver(measure);
    if (bounds.current) observer.observe(bounds.current);
    if (imageBox.current) observer.observe(imageBox.current);
    return () => observer.disconnect();
  }, [x, y]);
  return (
    <div className="relative flex min-h-0 flex-1 items-center justify-center">
      <div
        ref={bounds}
        className="flex h-full w-full items-center justify-center overflow-hidden rounded-lg"
      >
        <motion.div
          ref={imageBox}
          data-lightbox-content=""
          layoutId={layoutId}
          transition={SPRING_LAYOUT}
          className="relative flex max-h-full max-w-full items-center justify-center"
          style={{
            aspectRatio: `${image.width} / ${image.height}`,
            width: `min(100%, calc((100dvh - 160px) * ${image.width / image.height}))`,
          }}
        >
          <div className={cn("absolute inset-0", !zoom && "overflow-hidden")}>
            <AnimatePresence initial={false} custom={{ direction, reduce }}>
              <PresenceGate key={session}>
                {({ isPresent: slidePresent, gate }) => (
                  <motion.div
                    {...gate}
                    custom={{ direction, reduce }}
                    variants={imageSwapVariants}
                    initial="enter"
                    animate="visible"
                    exit="exit"
                    className="absolute inset-0"
                  >
                    {/* biome-ignore lint/performance/noImgElement: Copy-paste registry images must work outside Next.js. */}
                    <motion.img
                      src={image.src}
                      alt={image.alt}
                      width={image.width}
                      height={image.height}
                      draggable={false}
                      onError={() => {
                        if (slidePresent && present) setFailed(true);
                      }}
                      // Keep Motion's drag/layout registration mounted across zoom changes.
                      drag
                      dragListener={zoom}
                      dragConstraints={{
                        left: -pan.x,
                        right: pan.x,
                        top: -pan.y,
                        bottom: pan.y,
                      }}
                      dragElastic={0.08}
                      dragMomentum={false}
                      animate={{
                        scale: zoom && present ? 2 : 1,
                        ...(!present ? { x: 0, y: 0 } : {}),
                      }}
                      transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
                      onPointerDown={(event) => {
                        // Swiping the image keeps keyboard focus on the controls.
                        event.preventDefault();
                        if (!zoom && event.isPrimary)
                          swipe.current = {
                            x: event.clientX,
                            y: event.clientY,
                          };
                      }}
                      onPointerUp={(event) => {
                        const start = swipe.current;
                        swipe.current = null;
                        if (!start || zoom) return;
                        const dx = event.clientX - start.x;
                        const dy = event.clientY - start.y;
                        if (
                          Math.abs(dx) > 50 &&
                          Math.abs(dx) > Math.abs(dy) * 1.5
                        )
                          onSwipe(dx < 0 ? 1 : -1);
                      }}
                      onPointerCancel={() => {
                        swipe.current = null;
                      }}
                      className="block h-full w-full select-none object-contain"
                      style={{
                        x,
                        y,
                        touchAction: zoom ? "none" : "pan-y",
                        cursor: zoom ? "grab" : undefined,
                      }}
                    />
                  </motion.div>
                )}
              </PresenceGate>
            </AnimatePresence>
          </div>
          {failed && (
            <p
              role="status"
              className="absolute inset-0 flex items-center justify-center bg-neutral-900 p-4 text-center text-sm text-white"
            >
              Unable to load this image.
            </p>
          )}
        </motion.div>
      </div>
      <button
        type="button"
        aria-label={zoom ? "Zoom out" : "Zoom in"}
        aria-pressed={zoom}
        aria-describedby={panHelp}
        onKeyDown={(event) => {
          if (!zoom) return;
          const dx =
            event.key === "ArrowLeft"
              ? -40
              : event.key === "ArrowRight"
                ? 40
                : 0;
          const dy =
            event.key === "ArrowUp" ? -40 : event.key === "ArrowDown" ? 40 : 0;
          if (!dx && !dy) return;
          event.preventDefault();
          event.stopPropagation();
          x.set(Math.max(-pan.x, Math.min(pan.x, x.get() + dx)));
          y.set(Math.max(-pan.y, Math.min(pan.y, y.get() + dy)));
        }}
        onClick={() => {
          x.set(0);
          y.set(0);
          setZoom((current) => !current);
        }}
        className={cn(controlClass, "absolute bottom-3 right-3 bg-black/60")}
      >
        {zoom ? (
          <Minus size={18} aria-hidden="true" />
        ) : (
          <Plus size={18} aria-hidden="true" />
        )}
      </button>
      <span id={panHelp} className="sr-only">
        When zoomed in, drag the image to pan, or keep this control focused and
        use the arrow keys.
      </span>
    </div>
  );
}
