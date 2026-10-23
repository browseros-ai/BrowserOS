"use client";
// beui.dev/components/motion/animated-sidebar

import { ChevronRight } from "lucide-react";
import {
  AnimatePresence,
  type HTMLMotionProps,
  motion,
  useReducedMotion,
  type Variants,
} from "motion/react";
import {
  type ButtonHTMLAttributes,
  type CSSProperties,
  createContext,
  forwardRef,
  type HTMLAttributes,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { createPortal } from "react-dom";
import { SharedLayoutBg } from "@/components/motion/shared-layout-bg";
import {
  EASE_DRAWER,
  EASE_OUT,
  SPRING_LAYOUT,
  SPRING_PRESS,
} from "@/lib/ease";
import { cn } from "@/lib/utils";

type SidebarState = "expanded" | "collapsed";
type SidebarSide = "left" | "right";
type SidebarVariant = "sidebar" | "floating" | "inset";
type SidebarCollapsible = "offcanvas" | "icon" | "none";

const MOBILE_QUERY = "(max-width: 767px)";
const SIDEBAR_KEYBOARD_SHORTCUT = "b";

const PANEL_TRANSITION = {
  duration: 0.36,
  ease: EASE_DRAWER,
} as const;

// The desktop rail settles at a hard zero-width boundary. Keep the spring
// critically damped so it cannot overshoot, pause against that boundary, and
// then snap back during the final frame.
const SIDEBAR_MORPH_TRANSITION = {
  type: "spring",
  stiffness: 380,
  damping: 35,
  mass: 0.75,
} as const;

const LABEL_ENTER_TRANSITION = {
  duration: 0.2,
  delay: 0.08,
  ease: EASE_OUT,
} as const;

const LABEL_EXIT_TRANSITION = {
  duration: 0.12,
  ease: EASE_OUT,
} as const;

const SUBMENU_TRANSITION = {
  duration: 0.18,
  ease: EASE_OUT,
} as const;

const SUBMENU_VARIANTS: Variants = {
  closed: {
    opacity: 0,
    clipPath: "inset(0 0 100% 0 round 8px)",
    transition: {
      duration: 0.14,
      ease: EASE_OUT,
      staggerChildren: 0.025,
      staggerDirection: -1,
    },
  },
  open: {
    opacity: 1,
    clipPath: "inset(0 0 0% 0 round 8px)",
    transition: {
      duration: 0.2,
      delayChildren: 0.035,
      ease: EASE_OUT,
      staggerChildren: 0.045,
    },
  },
};

const SUBMENU_ITEM_VARIANTS: Variants = {
  closed: {
    opacity: 0,
    y: -6,
    filter: "blur(3px)",
  },
  open: {
    opacity: 1,
    y: 0,
    filter: "blur(0px)",
    transition: SUBMENU_TRANSITION,
  },
};

const REDUCED_TRANSITION = {
  duration: 0.16,
  ease: EASE_OUT,
} as const;

const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

function subscribeToMobileQuery(callback: () => void) {
  const query = window.matchMedia(MOBILE_QUERY);
  query.addEventListener("change", callback);
  return () => query.removeEventListener("change", callback);
}

function getMobileSnapshot() {
  return window.matchMedia(MOBILE_QUERY).matches;
}

function getServerMobileSnapshot() {
  return false;
}

function useIsMobile() {
  return useSyncExternalStore(
    subscribeToMobileQuery,
    getMobileSnapshot,
    getServerMobileSnapshot,
  );
}

interface AnimatedSidebarContextValue {
  isMobile: boolean;
  layoutId: string;
  open: boolean;
  openMobile: boolean;
  reduce: boolean;
  setOpen: (open: boolean) => void;
  setOpenMobile: (open: boolean) => void;
  state: SidebarState;
  toggleSidebar: () => void;
  triggerRef: React.RefObject<HTMLButtonElement | null>;
}

const AnimatedSidebarContext =
  createContext<AnimatedSidebarContextValue | null>(null);

interface AnimatedSidebarPanelContextValue {
  collapsed: boolean;
  collapsible: SidebarCollapsible;
  side: SidebarSide;
}

const AnimatedSidebarPanelContext =
  createContext<AnimatedSidebarPanelContextValue | null>(null);

export function useAnimatedSidebar() {
  const context = useContext(AnimatedSidebarContext);
  if (!context) {
    throw new Error(
      "useAnimatedSidebar must be used inside AnimatedSidebarProvider.",
    );
  }
  return context;
}

function useAnimatedSidebarPanel() {
  const context = useContext(AnimatedSidebarPanelContext);
  if (!context) {
    throw new Error(
      "Animated Sidebar parts must be used inside AnimatedSidebar.",
    );
  }
  return context;
}

type SidebarProviderStyle = CSSProperties & {
  "--sidebar-width"?: string;
  "--sidebar-width-icon"?: string;
  "--sidebar-width-mobile"?: string;
};

export interface AnimatedSidebarProviderProps
  extends HTMLAttributes<HTMLDivElement> {
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  openMobile?: boolean;
  defaultOpenMobile?: boolean;
  onOpenMobileChange?: (open: boolean) => void;
  style?: SidebarProviderStyle;
}

export function AnimatedSidebarProvider({
  children,
  open,
  defaultOpen = true,
  onOpenChange,
  openMobile,
  defaultOpenMobile = false,
  onOpenMobileChange,
  className,
  style,
  ...props
}: AnimatedSidebarProviderProps) {
  const [internalOpen, setInternalOpen] = useState(defaultOpen);
  const [internalOpenMobile, setInternalOpenMobile] =
    useState(defaultOpenMobile);
  const isMobile = useIsMobile();
  const reduce = useReducedMotion() ?? false;
  const generatedId = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const desktopOpen = open ?? internalOpen;
  const mobileOpen = openMobile ?? internalOpenMobile;

  const setOpen = useCallback(
    (nextOpen: boolean) => {
      if (open === undefined) setInternalOpen(nextOpen);
      onOpenChange?.(nextOpen);
    },
    [onOpenChange, open],
  );

  const setOpenMobile = useCallback(
    (nextOpen: boolean) => {
      if (openMobile === undefined) setInternalOpenMobile(nextOpen);
      onOpenMobileChange?.(nextOpen);
    },
    [onOpenMobileChange, openMobile],
  );

  const toggleSidebar = useCallback(() => {
    if (isMobile) setOpenMobile(!mobileOpen);
    else setOpen(!desktopOpen);
  }, [desktopOpen, isMobile, mobileOpen, setOpen, setOpenMobile]);

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      if (
        event.key.toLowerCase() === SIDEBAR_KEYBOARD_SHORTCUT &&
        (event.metaKey || event.ctrlKey)
      ) {
        event.preventDefault();
        toggleSidebar();
      }
    };

    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
  }, [toggleSidebar]);

  return (
    <AnimatedSidebarContext.Provider
      value={{
        isMobile,
        layoutId: `${generatedId}-active`,
        open: desktopOpen,
        openMobile: mobileOpen,
        reduce,
        setOpen,
        setOpenMobile,
        state: desktopOpen ? "expanded" : "collapsed",
        toggleSidebar,
        triggerRef,
      }}
    >
      <div
        {...props}
        data-slot="sidebar-wrapper"
        data-state={desktopOpen ? "expanded" : "collapsed"}
        style={{
          "--sidebar-width": "16rem",
          "--sidebar-width-icon": "4.25rem",
          "--sidebar-width-mobile": "18rem",
          ...style,
        }}
        className={cn(
          "group/sidebar-wrapper flex min-h-svh w-full min-w-0",
          className,
        )}
      >
        {children}
      </div>
    </AnimatedSidebarContext.Provider>
  );
}

function MobileSidebar({
  ariaLabel,
  children,
  className,
  side,
}: {
  ariaLabel: string;
  children: ReactNode;
  className?: string;
  side: SidebarSide;
}) {
  const context = useAnimatedSidebar();
  const panelRef = useRef<HTMLDivElement>(null);
  const [mounted, setMounted] = useState(false);
  // The sheet is mounted for as long as the viewport is mobile, so it hides
  // itself while closed rather than sitting there transparent and interactive.
  // Opening shows it in the same commit that starts the slide — a delayed show
  // would run the focus effect below against a still-hidden panel, and focus()
  // on a hidden element is ignored. Closing waits for the slide to finish, and
  // the panel's own exit tells us when that is: no duration to keep in sync.
  const [hidden, setHidden] = useState(!context.openMobile);
  // The completion callback fires for the open slide too, and it reads state
  // from whenever motion settles: a ref keeps it on the current one.
  const openMobileRef = useRef(context.openMobile);

  useEffect(() => setMounted(true), []);

  useEffect(() => {
    openMobileRef.current = context.openMobile;
    if (context.openMobile) setHidden(false);
  }, [context.openMobile]);

  useEffect(() => {
    if (!context.openMobile) return;

    const body = document.body;
    const scrollY = window.scrollY;
    const previousBodyStyles = {
      left: body.style.left,
      overflow: body.style.overflow,
      position: body.style.position,
      right: body.style.right,
      top: body.style.top,
    };

    body.style.position = "fixed";
    body.style.top = `-${scrollY}px`;
    body.style.left = "0";
    body.style.right = "0";
    body.style.overflow = "hidden";

    const focusFrame = requestAnimationFrame(() => {
      const firstFocusable =
        panelRef.current?.querySelector<HTMLElement>(FOCUSABLE_SELECTOR);
      (firstFocusable ?? panelRef.current)?.focus({ preventScroll: true });
    });

    return () => {
      cancelAnimationFrame(focusFrame);
      body.style.position = previousBodyStyles.position;
      body.style.top = previousBodyStyles.top;
      body.style.left = previousBodyStyles.left;
      body.style.right = previousBodyStyles.right;
      body.style.overflow = previousBodyStyles.overflow;
      window.scrollTo(0, scrollY);
      context.triggerRef.current?.focus({ preventScroll: true });
    };
  }, [context.openMobile, context.triggerRef]);

  if (!mounted) return null;

  // This container groups the sheet for hiding and the z-index and carries no
  // box: both children are `fixed` and resolve against the viewport themselves.
  // The scrim spans the viewport edges but paints a colour, and the panel is
  // inset off one side and paints its own surface, so no layer here is a
  // transparent edge-spanning one.
  return createPortal(
    <div
      className={cn(
        "pointer-events-none fixed left-0 top-0 z-50 size-0 md:hidden",
        hidden && !context.openMobile ? "invisible" : "visible",
      )}
    >
      <motion.button
        type="button"
        aria-label="Close sidebar"
        tabIndex={context.openMobile ? 0 : -1}
        initial={false}
        animate={{ opacity: context.openMobile ? 1 : 0 }}
        transition={
          context.reduce ? REDUCED_TRANSITION : PANEL_TRANSITION
        }
        onClick={() => context.setOpenMobile(false)}
        className={cn(
          "fixed inset-0 bg-black/40",
          context.openMobile
            ? "pointer-events-auto"
            : "pointer-events-none",
        )}
      />

      <motion.div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={ariaLabel}
        aria-hidden={!context.openMobile}
        inert={!context.openMobile}
        tabIndex={-1}
        data-mobile="true"
        data-state={context.openMobile ? "expanded" : "collapsed"}
        data-side={side}
        initial={false}
        animate={{
          opacity: context.reduce
            ? context.openMobile
              ? 1
              : 0
            : 1,
          x: context.reduce
            ? 0
            : context.openMobile
              ? "0%"
              : side === "left"
                ? "-100%"
                : "100%",
        }}
        transition={
          context.reduce ? REDUCED_TRANSITION : PANEL_TRANSITION
        }
        onAnimationComplete={() => {
          if (!openMobileRef.current) setHidden(true);
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            context.setOpenMobile(false);
            return;
          }

          if (event.key !== "Tab") return;
          const focusable = panelRef.current
            ? Array.from(
                panelRef.current.querySelectorAll<HTMLElement>(
                  FOCUSABLE_SELECTOR,
                ),
              )
            : [];

          if (focusable.length === 0) {
            event.preventDefault();
            panelRef.current?.focus();
            return;
          }

          const first = focusable[0];
          const last = focusable[focusable.length - 1];
          if (event.shiftKey && document.activeElement === first) {
            event.preventDefault();
            last.focus();
          } else if (!event.shiftKey && document.activeElement === last) {
            event.preventDefault();
            first.focus();
          }
        }}
        className={cn(
          "pointer-events-auto fixed inset-y-0 flex h-dvh w-(--sidebar-width-mobile) max-w-[88vw] flex-col overflow-hidden",
          "border-border bg-background shadow-2xl will-change-transform",
          side === "left" ? "left-0 border-r" : "right-0 border-l",
          !context.openMobile && "pointer-events-none",
          className,
        )}
      >
        <AnimatedSidebarPanelContext.Provider
          value={{ collapsed: false, collapsible: "none", side }}
        >
          {children}
        </AnimatedSidebarPanelContext.Provider>
      </motion.div>
    </div>,
    document.body,
  );
}

export interface AnimatedSidebarProps
  extends Omit<HTMLMotionProps<"aside">, "children"> {
  children?: ReactNode;
  side?: SidebarSide;
  variant?: SidebarVariant;
  collapsible?: SidebarCollapsible;
  ariaLabel?: string;
  panelClassName?: string;
}

export const AnimatedSidebar = forwardRef<HTMLElement, AnimatedSidebarProps>(
  function AnimatedSidebar(
    {
      side = "left",
      variant = "sidebar",
      collapsible = "icon",
      ariaLabel = "Sidebar",
      children,
      className,
      panelClassName,
      style,
      ...props
    },
    forwardedRef,
  ) {
    const context = useAnimatedSidebar();
    const collapsed = collapsible !== "none" && !context.open;
    const offcanvas = collapsed && collapsible === "offcanvas";
    const width = offcanvas
      ? "0px"
      : collapsed
        ? "var(--sidebar-width-icon)"
        : "var(--sidebar-width)";

    if (context.isMobile) {
      return (
        <MobileSidebar
          ariaLabel={ariaLabel}
          className={className}
          side={side}
        >
          {children}
        </MobileSidebar>
      );
    }

    return (
      <motion.aside
        {...props}
        ref={forwardedRef}
        initial={false}
        aria-label={ariaLabel}
        data-slot="sidebar"
        data-state={collapsed ? "collapsed" : "expanded"}
        data-collapsible={collapsible}
        data-variant={variant}
        data-side={side}
        animate={{ width }}
        transition={
          context.reduce ? { duration: 0 } : SIDEBAR_MORPH_TRANSITION
        }
        style={style}
        className={cn(
          "group/sidebar relative hidden h-auto shrink-0 md:block will-change-[width]",
          "peer",
          side === "right" && "order-last",
          className,
        )}
      >
        <motion.div
          initial={false}
          animate={{
            opacity: offcanvas ? 0 : 1,
            x: offcanvas ? (side === "left" ? "-100%" : "100%") : "0%",
          }}
          transition={
            context.reduce ? REDUCED_TRANSITION : PANEL_TRANSITION
          }
          className={cn(
            "sticky top-0 flex h-svh w-full flex-col overflow-hidden bg-background",
            collapsible === "offcanvas" && "w-[var(--sidebar-width)]",
            variant === "sidebar" &&
              (side === "left" ? "border-border border-r" : "border-border border-l"),
            variant === "floating" &&
              "m-2 h-[calc(100svh-1rem)] rounded-2xl border border-border shadow-sm",
            variant === "inset" && "m-2 h-[calc(100svh-1rem)] rounded-2xl",
            panelClassName,
          )}
        >
          <AnimatedSidebarPanelContext.Provider
            value={{ collapsed, collapsible, side }}
          >
            {children}
          </AnimatedSidebarPanelContext.Provider>
        </motion.div>
      </motion.aside>
    );
  },
);

export interface AnimatedSidebarTriggerProps
  extends ButtonHTMLAttributes<HTMLButtonElement> {}

export const AnimatedSidebarTrigger = forwardRef<
  HTMLButtonElement,
  AnimatedSidebarTriggerProps
>(function AnimatedSidebarTrigger(
  { className, onClick, type = "button", ...props },
  forwardedRef,
) {
  const context = useAnimatedSidebar();
  const expanded = context.isMobile ? context.openMobile : context.open;

  return (
    <button
      {...props}
      ref={(node) => {
        context.triggerRef.current = node;
        if (typeof forwardedRef === "function") forwardedRef(node);
        else if (forwardedRef) forwardedRef.current = node;
      }}
      type={type}
      aria-label={props["aria-label"] ?? "Toggle sidebar"}
      aria-expanded={expanded}
      data-slot="sidebar-trigger"
      data-state={expanded ? "expanded" : "collapsed"}
      onClick={(event) => {
        onClick?.(event);
        if (!event.defaultPrevented) context.toggleSidebar();
      }}
      className={cn(
        "inline-flex size-10 shrink-0 items-center justify-center rounded-xl outline-none",
        "focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
        className,
      )}
    />
  );
});

export interface AnimatedSidebarCloseProps
  extends ButtonHTMLAttributes<HTMLButtonElement> {}

export const AnimatedSidebarClose = forwardRef<
  HTMLButtonElement,
  AnimatedSidebarCloseProps
>(function AnimatedSidebarClose(
  { className, onClick, type = "button", ...props },
  forwardedRef,
) {
  const context = useAnimatedSidebar();

  return (
    <button
      {...props}
      ref={forwardedRef}
      type={type}
      aria-label={props["aria-label"] ?? "Close sidebar"}
      onClick={(event) => {
        onClick?.(event);
        if (event.defaultPrevented) return;
        if (context.isMobile) context.setOpenMobile(false);
        else context.setOpen(false);
      }}
      className={cn(
        "inline-flex size-10 shrink-0 items-center justify-center rounded-xl outline-none",
        "focus-visible:ring-2 focus-visible:ring-ring",
        className,
      )}
    />
  );
});

export interface AnimatedSidebarRailProps
  extends ButtonHTMLAttributes<HTMLButtonElement> {}

export const AnimatedSidebarRail = forwardRef<
  HTMLButtonElement,
  AnimatedSidebarRailProps
>(function AnimatedSidebarRail(
  { className, onClick, type = "button", ...props },
  forwardedRef,
) {
  const context = useAnimatedSidebar();
  const panel = useAnimatedSidebarPanel();

  return (
    <button
      {...props}
      ref={forwardedRef}
      type={type}
      data-side={panel.side}
      aria-label={props["aria-label"] ?? "Toggle sidebar"}
      title="Toggle sidebar"
      tabIndex={-1}
      onClick={(event) => {
        onClick?.(event);
        if (!event.defaultPrevented) context.toggleSidebar();
      }}
      className={cn(
        "absolute inset-y-0 z-20 hidden w-4 -translate-x-1/2 outline-none md:block",
        "after:absolute after:inset-y-0 after:left-1/2 after:w-px after:bg-transparent after:transition-colors hover:after:bg-border",
        "data-[side=right]:right-0 data-[side=right]:translate-x-1/2 data-[side=left]:left-full",
        className,
      )}
    />
  );
});

export interface AnimatedSidebarInsetProps
  extends HTMLMotionProps<"main"> {}

export const AnimatedSidebarInset = forwardRef<
  HTMLElement,
  AnimatedSidebarInsetProps
>(function AnimatedSidebarInset({ className, ...props }, forwardedRef) {
  return (
    <motion.main
      {...props}
      ref={forwardedRef}
      data-slot="sidebar-inset"
      className={cn(
        "relative flex min-h-svh min-w-0 flex-1 flex-col bg-background",
        "md:peer-data-[variant=inset]:m-2 md:peer-data-[variant=inset]:ml-0 md:peer-data-[variant=inset]:rounded-2xl md:peer-data-[variant=inset]:shadow-sm",
        className,
      )}
    />
  );
});

export const AnimatedSidebarHeader = forwardRef<
  HTMLDivElement,
  HTMLAttributes<HTMLDivElement>
>(function AnimatedSidebarHeader({ className, ...props }, forwardedRef) {
  return (
    <div
      {...props}
      ref={forwardedRef}
      data-slot="sidebar-header"
      className={cn("flex shrink-0 flex-col gap-2 p-3", className)}
    />
  );
});

export const AnimatedSidebarContent = forwardRef<
  HTMLDivElement,
  HTMLAttributes<HTMLDivElement>
>(function AnimatedSidebarContent({ className, ...props }, forwardedRef) {
  return (
    <div
      {...props}
      ref={forwardedRef}
      data-slot="sidebar-content"
      className={cn(
        "flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto overflow-x-hidden overscroll-contain px-2 py-2",
        className,
      )}
    />
  );
});

export const AnimatedSidebarFooter = forwardRef<
  HTMLDivElement,
  HTMLAttributes<HTMLDivElement>
>(function AnimatedSidebarFooter({ className, ...props }, forwardedRef) {
  return (
    <div
      {...props}
      ref={forwardedRef}
      data-slot="sidebar-footer"
      className={cn(
        "flex shrink-0 flex-col gap-2 border-border border-t p-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]",
        className,
      )}
    />
  );
});

export const AnimatedSidebarGroup = forwardRef<
  HTMLDivElement,
  HTMLAttributes<HTMLDivElement>
>(function AnimatedSidebarGroup({ className, ...props }, forwardedRef) {
  return (
    <div
      {...props}
      ref={forwardedRef}
      data-slot="sidebar-group"
      className={cn("flex w-full min-w-0 flex-col px-1 py-1.5", className)}
    />
  );
});

export const AnimatedSidebarGroupLabel = forwardRef<
  HTMLDivElement,
  HTMLAttributes<HTMLDivElement>
>(function AnimatedSidebarGroupLabel(
  { children, className, ...props },
  forwardedRef,
) {
  const { collapsed } = useAnimatedSidebarPanel();

  return (
    <div
      {...props}
      ref={forwardedRef}
      aria-hidden={collapsed}
      data-slot="sidebar-group-label"
      className={cn(
        "mb-1 h-7 overflow-hidden px-2 text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground transition-opacity",
        collapsed ? "opacity-0" : "opacity-100",
        className,
      )}
    >
      {children}
    </div>
  );
});

export const AnimatedSidebarGroupContent = forwardRef<
  HTMLDivElement,
  HTMLAttributes<HTMLDivElement>
>(function AnimatedSidebarGroupContent(
  { className, ...props },
  forwardedRef,
) {
  return (
    <div
      {...props}
      ref={forwardedRef}
      data-slot="sidebar-group-content"
      className={cn("w-full min-w-0", className)}
    />
  );
});

export const AnimatedSidebarMenu = forwardRef<
  HTMLUListElement,
  HTMLAttributes<HTMLUListElement>
>(function AnimatedSidebarMenu(
  { children, className, ...props },
  forwardedRef,
) {
  return (
    <SharedLayoutBg
      {...props}
      ref={forwardedRef as React.Ref<HTMLElement>}
      as="ul"
      inset={0}
      pillClassName="rounded-xl bg-muted/70"
      pillContainerClassName="inset-y-auto top-0 h-9"
      data-slot="sidebar-menu"
      className={cn("flex w-full min-w-0 list-none flex-col gap-0.5", className)}
    >
      {children}
    </SharedLayoutBg>
  );
});

export const AnimatedSidebarMenuItem = forwardRef<
  HTMLLIElement,
  HTMLMotionProps<"li">
>(function AnimatedSidebarMenuItem({ className, ...props }, forwardedRef) {
  return (
    <motion.li
      {...props}
      ref={forwardedRef}
      layout="position"
      transition={SPRING_LAYOUT}
      data-slot="sidebar-menu-item"
      className={cn("relative", className)}
    />
  );
});

export interface AnimatedSidebarMenuSubProps
  extends Omit<HTMLMotionProps<"ul">, "children"> {
  open: boolean;
  children?: ReactNode;
}

export const AnimatedSidebarMenuSub = forwardRef<
  HTMLUListElement,
  AnimatedSidebarMenuSubProps
>(function AnimatedSidebarMenuSub(
  { open, children, className, ...props },
  forwardedRef,
) {
  const context = useAnimatedSidebar();
  const panel = useAnimatedSidebarPanel();

  return (
    <AnimatePresence initial={false} mode="popLayout">
      {open && !panel.collapsed ? (
        <motion.ul
          {...props}
          ref={forwardedRef}
          key="sidebar-submenu"
          variants={context.reduce ? undefined : SUBMENU_VARIANTS}
          initial={context.reduce ? false : "closed"}
          animate={context.reduce ? { opacity: 1 } : "open"}
          exit={context.reduce ? { opacity: 0 } : "closed"}
          transition={context.reduce ? { duration: 0.12 } : undefined}
          data-slot="sidebar-menu-sub"
          className={cn(
            "relative mt-1 ml-5 flex min-w-0 flex-col gap-0.5 border-border border-l pl-3",
            className,
          )}
        >
          {children}
        </motion.ul>
      ) : null}
    </AnimatePresence>
  );
});

export const AnimatedSidebarMenuSubItem = forwardRef<
  HTMLLIElement,
  HTMLMotionProps<"li">
>(function AnimatedSidebarMenuSubItem(
  { className, ...props },
  forwardedRef,
) {
  return (
    <motion.li
      {...props}
      ref={forwardedRef}
      variants={SUBMENU_ITEM_VARIANTS}
      data-slot="sidebar-menu-sub-item"
      className={cn("relative min-w-0", className)}
    />
  );
});

export interface AnimatedSidebarMenuSubButtonProps {
  children: ReactNode;
  icon?: ReactNode;
  href?: string;
  isActive?: boolean;
  disabled?: boolean;
  closeOnSelect?: boolean;
  target?: "_blank" | "_self" | "_parent" | "_top";
  rel?: string;
  onSelect?: () => void;
  className?: string;
}

export function AnimatedSidebarMenuSubButton({
  children,
  icon,
  href,
  isActive = false,
  disabled = false,
  closeOnSelect = true,
  target,
  rel,
  onSelect,
  className,
}: AnimatedSidebarMenuSubButtonProps) {
  const context = useAnimatedSidebar();

  const select = (
    event: React.MouseEvent<HTMLAnchorElement | HTMLButtonElement>,
  ) => {
    if (disabled) {
      event.preventDefault();
      return;
    }
    onSelect?.();
    if (context.isMobile && closeOnSelect) context.setOpenMobile(false);
  };

  const content = (
    <>
      <span
        aria-hidden="true"
        className="grid size-4 shrink-0 place-items-center"
      >
        {icon ?? <span className="size-1 rounded-full bg-current" />}
      </span>
      <span className="min-w-0 flex-1 truncate">{children}</span>
    </>
  );

  const interactiveClassName = cn(
    "flex min-h-8 w-full min-w-0 items-center gap-2 rounded-lg px-2 text-left text-xs outline-none",
    "text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground",
    "focus-visible:bg-muted/70 focus-visible:ring-2 focus-visible:ring-ring",
    isActive && "bg-muted/70 text-foreground",
    disabled && "cursor-not-allowed opacity-40",
    className,
  );

  return href ? (
    <motion.a
      href={href}
      target={target}
      rel={
        rel ??
        (target === "_blank" ? "noreferrer noopener" : undefined)
      }
      aria-current={isActive ? "page" : undefined}
      aria-disabled={disabled || undefined}
      tabIndex={disabled ? -1 : undefined}
      onClick={select}
      whileTap={context.reduce || disabled ? undefined : { scale: 0.98 }}
      transition={SPRING_PRESS}
      className={interactiveClassName}
    >
      {content}
    </motion.a>
  ) : (
    <motion.button
      type="button"
      disabled={disabled}
      aria-current={isActive ? "page" : undefined}
      onClick={select}
      whileTap={context.reduce || disabled ? undefined : { scale: 0.98 }}
      transition={SPRING_PRESS}
      className={interactiveClassName}
    >
      {content}
    </motion.button>
  );
}

export interface AnimatedSidebarMenuButtonProps {
  children: ReactNode;
  icon?: ReactNode;
  badge?: ReactNode;
  href?: string;
  isActive?: boolean;
  ariaExpanded?: boolean;
  disabled?: boolean;
  closeOnSelect?: boolean;
  target?: "_blank" | "_self" | "_parent" | "_top";
  rel?: string;
  onSelect?: () => void;
  className?: string;
}

export function AnimatedSidebarMenuButton({
  children,
  icon,
  badge,
  href,
  isActive = false,
  ariaExpanded,
  disabled = false,
  closeOnSelect,
  target,
  rel,
  onSelect,
  className,
}: AnimatedSidebarMenuButtonProps) {
  const context = useAnimatedSidebar();
  const panel = useAnimatedSidebarPanel();
  const textLabel = typeof children === "string" ? children : undefined;

  const select = (
    event: React.MouseEvent<HTMLAnchorElement | HTMLButtonElement>,
  ) => {
    if (disabled) {
      event.preventDefault();
      return;
    }
    onSelect?.();
    const shouldCloseOnSelect =
      closeOnSelect ?? ariaExpanded === undefined;
    if (context.isMobile && shouldCloseOnSelect) {
      context.setOpenMobile(false);
    }
    // A submenu cannot render in the icon rail, so opening one from there
    // leaves its children unreachable — a pointer can still fall back to the
    // rail or the shortcut, a finger has nothing. Selecting a group unfolds
    // the panel that is about to hold it.
    if (ariaExpanded !== undefined && panel.collapsed && !context.isMobile) {
      context.setOpen(true);
    }
  };

  const content = (
    <>
      {isActive ? (
        <motion.span
          layoutId={context.layoutId}
          transition={context.reduce ? { duration: 0 } : SPRING_LAYOUT}
          className="absolute inset-0 rounded-xl bg-muted"
        />
      ) : null}
      {icon ? (
        <span
          aria-hidden="true"
          className="relative z-10 grid size-5 shrink-0 place-items-center"
        >
          {icon}
        </span>
      ) : null}
      <motion.span
        initial={false}
        animate={{
          opacity: panel.collapsed ? 0 : 1,
          x: panel.collapsed ? -4 : 0,
        }}
        transition={
          context.reduce
            ? REDUCED_TRANSITION
            : panel.collapsed
              ? LABEL_EXIT_TRANSITION
              : LABEL_ENTER_TRANSITION
        }
        aria-hidden={panel.collapsed}
        className={cn(
          "relative z-10 min-w-0 flex-1 truncate",
          panel.collapsed && "pointer-events-none",
        )}
      >
        {children}
      </motion.span>
      {badge && !panel.collapsed ? (
        <span className="relative z-10 shrink-0 text-xs text-muted-foreground">
          {badge}
        </span>
      ) : null}
      {ariaExpanded !== undefined ? (
        <motion.span
          aria-hidden="true"
          initial={false}
          animate={{
            opacity: panel.collapsed ? 0 : 1,
            rotate: ariaExpanded ? 90 : 0,
            x: panel.collapsed ? 4 : 0,
          }}
          transition={context.reduce ? { duration: 0 } : SPRING_LAYOUT}
          className="relative z-10 grid size-4 shrink-0 place-items-center text-muted-foreground"
        >
          <ChevronRight className="size-3.5" />
        </motion.span>
      ) : null}
    </>
  );

  const interactiveClassName = cn(
    "relative flex min-h-9 w-full min-w-0 items-center gap-2.5 overflow-hidden rounded-xl px-3 text-left text-sm font-medium outline-none",
    "text-muted-foreground transition-colors hover:text-foreground",
    "focus-visible:bg-muted/70 focus-visible:ring-2 focus-visible:ring-ring",
    isActive && "text-foreground",
    disabled && "cursor-not-allowed opacity-40",
    className,
  );

  return href ? (
    <motion.a
      href={href}
      target={target}
      rel={
        rel ??
        (target === "_blank" ? "noreferrer noopener" : undefined)
      }
      aria-current={isActive ? "page" : undefined}
      aria-expanded={ariaExpanded}
      aria-disabled={disabled || undefined}
      aria-label={panel.collapsed ? textLabel : undefined}
      title={panel.collapsed ? textLabel : undefined}
      tabIndex={disabled ? -1 : undefined}
      onClick={select}
      whileTap={context.reduce || disabled ? undefined : { scale: 0.98 }}
      transition={SPRING_PRESS}
      className={interactiveClassName}
    >
      {content}
    </motion.a>
  ) : (
    <motion.button
      type="button"
      disabled={disabled}
      aria-current={isActive ? "page" : undefined}
      aria-expanded={ariaExpanded}
      aria-label={panel.collapsed ? textLabel : undefined}
      title={panel.collapsed ? textLabel : undefined}
      onClick={select}
      whileTap={context.reduce || disabled ? undefined : { scale: 0.98 }}
      transition={SPRING_PRESS}
      className={interactiveClassName}
    >
      {content}
    </motion.button>
  );
}
