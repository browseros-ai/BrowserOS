import { Server } from 'lucide-react'
import type { FC } from 'react'
import AirtableSvg from '@/assets/mcp-icons/airtable.svg'
import AsanaSvg from '@/assets/mcp-icons/asana.svg'
import BoxSvg from '@/assets/mcp-icons/box.svg'
import BraveSearchSvg from '@/assets/mcp-icons/brave_search.svg'
import CalComSvg from '@/assets/mcp-icons/cal_com.svg'
import ClickUpSvg from '@/assets/mcp-icons/clickup.svg'
import CloudflareSvg from '@/assets/mcp-icons/cloudflare.svg'
import ConfluenceSvg from '@/assets/mcp-icons/confluence.svg'
import DiscordSvg from '@/assets/mcp-icons/discord.svg'
import DropboxSvg from '@/assets/mcp-icons/dropbox.svg'
import FigmaSvg from '@/assets/mcp-icons/figma.svg'
import GithubSvg from '@/assets/mcp-icons/github.svg'
import GitlabSvg from '@/assets/mcp-icons/gitlab.svg'
import GmailSvg from '@/assets/mcp-icons/gmail.svg'
import GoogleSvg from '@/assets/mcp-icons/google.svg'
import GoogleCalendarSvg from '@/assets/mcp-icons/google_calendar.svg'
import GoogleDocsSvg from '@/assets/mcp-icons/google_docs_editors.svg'
import GoogleDriveSvg from '@/assets/mcp-icons/google_drive.svg'
import GoogleFormsSvg from '@/assets/mcp-icons/google_forms.svg'
import HubSpotSvg from '@/assets/mcp-icons/hubspot.svg'
import IntercomSvg from '@/assets/mcp-icons/intercom.svg'
import JiraSvg from '@/assets/mcp-icons/jira.svg'
import LinearSvg from '@/assets/mcp-icons/linear.svg'
import LinkedinSvg from '@/assets/mcp-icons/linkedin.svg'
import Mem0Webp from '@/assets/mcp-icons/memo.webp'
import MicrosoftTeamsSvg from '@/assets/mcp-icons/microsoft_teams.svg'
import MixpanelSvg from '@/assets/mcp-icons/mixpanel.svg'
import MondaySvg from '@/assets/mcp-icons/monday.svg'
import NotionSvg from '@/assets/mcp-icons/notion.svg'
import OneDriveSvg from '@/assets/mcp-icons/onedrive.svg'
import OutlookSvg from '@/assets/mcp-icons/outlook.svg'
import PostHogSvg from '@/assets/mcp-icons/posthog.svg'
import PostmanSvg from '@/assets/mcp-icons/postman.svg'
import ResendSvg from '@/assets/mcp-icons/resend.svg'
import SalesforceSvg from '@/assets/mcp-icons/salesforce.svg'
import ShopifySvg from '@/assets/mcp-icons/shopify.svg'
import SlackSvg from '@/assets/mcp-icons/slack.svg'
import StripeSvg from '@/assets/mcp-icons/stripe.svg'
import SupabaseSvg from '@/assets/mcp-icons/supabase.svg'
import VercelSvg from '@/assets/mcp-icons/vercel.svg'
import WhatsAppWebp from '@/assets/mcp-icons/whatsapp.webp'
import WordPressSvg from '@/assets/mcp-icons/wordpress.svg'
import YouTubeSvg from '@/assets/mcp-icons/youtube.svg'
import ZendeskSvg from '@/assets/mcp-icons/zendesk.svg'

const mcpIconMap: Record<string, string> = {
  Gmail: GmailSvg,
  'Google Calendar': GoogleCalendarSvg,
  'Google Docs': GoogleDocsSvg,
  'Google Drive': GoogleDriveSvg,
  'Google Sheets': GoogleSvg,
  'Google Forms': GoogleFormsSvg,
  Slack: SlackSvg,
  LinkedIn: LinkedinSvg,
  Notion: NotionSvg,
  Airtable: AirtableSvg,
  Confluence: ConfluenceSvg,
  GitHub: GithubSvg,
  GitLab: GitlabSvg,
  Linear: LinearSvg,
  Jira: JiraSvg,
  Figma: FigmaSvg,
  Salesforce: SalesforceSvg,
  ClickUp: ClickUpSvg,
  Asana: AsanaSvg,
  Monday: MondaySvg,
  'Microsoft Teams': MicrosoftTeamsSvg,
  'Outlook Mail': OutlookSvg,
  'Outlook Calendar': OutlookSvg,
  Supabase: SupabaseSvg,
  Vercel: VercelSvg,
  Postman: PostmanSvg,
  Stripe: StripeSvg,
  Cloudflare: CloudflareSvg,
  'Brave Search': BraveSearchSvg,
  Mem0: Mem0Webp,
  Dropbox: DropboxSvg,
  OneDrive: OneDriveSvg,
  WordPress: WordPressSvg,
  YouTube: YouTubeSvg,
  Box: BoxSvg,
  HubSpot: HubSpotSvg,
  PostHog: PostHogSvg,
  Mixpanel: MixpanelSvg,
  Discord: DiscordSvg,
  WhatsApp: WhatsAppWebp,
  Shopify: ShopifySvg,
  'Cal.com': CalComSvg,
  Resend: ResendSvg,
  Zendesk: ZendeskSvg,
  Intercom: IntercomSvg,
}

export interface McpServerIconProps {
  serverName: string
  size?: number
  className?: string
}

export const McpServerIcon: FC<McpServerIconProps> = ({
  serverName,
  size = 20,
  className,
}) => {
  const iconSrc = mcpIconMap[serverName]

  if (iconSrc) {
    return (
      <img
        src={iconSrc}
        alt={serverName}
        width={size}
        height={size}
        className={`rounded-md ${className ?? ''}`}
      />
    )
  }

  return <Server size={size} className={className} />
}
