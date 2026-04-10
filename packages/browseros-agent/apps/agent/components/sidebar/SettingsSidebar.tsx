import {
  ArrowLeft,
  BookOpen,
  Bot,
  Compass,
  CreditCard,
  MessageSquare,
  Palette,
  RotateCcw,
  Search,
  Server,
  ShieldAlert,
  ShieldCheck,
} from 'lucide-react'
import type { FC } from 'react'
import { NavLink } from 'react-router'
import { ThemeToggle } from '@/components/elements/theme-toggle'
import { Feature } from '@/lib/browseros/capabilities'
import { useCapabilities } from '@/lib/browseros/useCapabilities'
import { cn } from '@/lib/utils'

type BaseNavItem = {
  name: string
  icon: typeof Bot
  feature?: Feature
}

type InternalNavItem = BaseNavItem & {
  href?: never
  to: string
}

type ExternalNavItem = BaseNavItem & {
  href: string
  to?: never
}

type NavItem = InternalNavItem | ExternalNavItem

type NavSection = {
  label: string
  items: NavItem[]
}

function isExternalNavItem(item: NavItem): item is ExternalNavItem {
  return 'href' in item
}

const getNavLinkClassName = (isActive: boolean) =>
  cn(
    'flex h-9 items-center gap-2 overflow-hidden whitespace-nowrap rounded-md px-3 font-medium text-sm transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
    isActive && 'bg-sidebar-accent text-sidebar-accent-foreground',
  )

const getSectionClassName = (index: number) =>
  cn(index > 0 && 'mt-3 border-t pt-3')

const sectionLabelClassName =
  'mb-2 px-3 font-semibold text-[10px] text-muted-foreground uppercase tracking-[0.18em]'

const primarySettingsSections: NavSection[] = [
  {
    label: 'Provider Settings',
    items: [
      { name: 'BrowserOS AI', to: '/settings/ai', icon: Bot },
      {
        name: 'Chat & Council Provider',
        to: '/settings/chat',
        icon: MessageSquare,
      },
      { name: 'Search Provider', to: '/settings/search', icon: Search },
    ],
  },
  {
    label: 'Other',
    items: [
      {
        name: 'Customize BrowserOS',
        to: '/settings/customization',
        icon: Palette,
        feature: Feature.CUSTOMIZATION_SUPPORT,
      },
      { name: 'Tool Approvals', to: '/settings/approvals', icon: ShieldCheck },
      { name: 'BrowserOS as MCP', to: '/settings/mcp', icon: Server },
      { name: 'ACL Rules', to: '/settings/acl', icon: ShieldAlert },
      {
        name: 'Usage & Billing',
        to: '/settings/usage',
        icon: CreditCard,
        feature: Feature.CREDITS_SUPPORT,
      },
    ],
  },
]

const helpItems: NavItem[] = [
  { name: 'Docs', href: 'https://docs.browseros.com/', icon: BookOpen },
  { name: 'Features', to: '/onboarding/features', icon: Compass },
  { name: 'Revisit Onboarding', to: '/onboarding', icon: RotateCcw },
]

export const SettingsSidebar: FC = () => {
  const { supports } = useCapabilities()

  const filteredSections = primarySettingsSections
    .map((section) => ({
      ...section,
      items: section.items.filter(
        (item) => !item.feature || supports(item.feature),
      ),
    }))
    .filter((section) => section.items.length > 0)

  const filteredHelpItems = helpItems.filter(
    (item) => !item.feature || supports(item.feature),
  )

  const renderNavItem = (item: NavItem) => {
    const Icon = item.icon

    if (isExternalNavItem(item)) {
      return (
        <a
          key={item.href}
          href={item.href}
          target="_blank"
          rel="noopener noreferrer"
          className={getNavLinkClassName(false)}
        >
          <Icon className="size-4 shrink-0" />
          <span className="truncate">{item.name}</span>
        </a>
      )
    }

    return (
      <NavLink
        key={item.to}
        to={item.to}
        end
        className={({ isActive }) => getNavLinkClassName(isActive)}
      >
        <Icon className="size-4 shrink-0" />
        <span className="truncate">{item.name}</span>
      </NavLink>
    )
  }

  const renderSection = (section: NavSection, index: number) => (
    <div key={section.label} className={getSectionClassName(index)}>
      <div className={sectionLabelClassName}>{section.label}</div>
      <nav className="space-y-1">{section.items.map(renderNavItem)}</nav>
    </div>
  )

  return (
    <div className="flex h-full w-64 flex-col border-r bg-sidebar text-sidebar-foreground">
      <div className="flex h-14 items-center justify-between border-b px-2">
        <NavLink
          to="/home"
          className="flex h-9 items-center gap-2 overflow-hidden whitespace-nowrap rounded-md px-3 font-medium text-sm transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
        >
          <ArrowLeft className="size-4 shrink-0" />
          <span className="truncate">Back</span>
        </NavLink>
        <ThemeToggle
          className="mr-1 h-8 w-8 shrink-0"
          iconClassName="h-4 w-4"
        />
      </div>

      <div className="flex flex-1 flex-col overflow-y-auto overflow-x-hidden p-2">
        <div className="mb-2 px-3 font-semibold text-muted-foreground text-xs uppercase tracking-wider">
          Settings
        </div>
        <div>{filteredSections.map(renderSection)}</div>
        <div className="mt-auto pt-4">
          <div className={sectionLabelClassName}>Help</div>
          <nav className="space-y-1">
            {filteredHelpItems.map(renderNavItem)}
          </nav>
        </div>
      </div>
    </div>
  )
}
