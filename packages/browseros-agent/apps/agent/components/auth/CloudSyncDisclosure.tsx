import { cloudSyncSignInLinks } from '@/lib/constants/productUrls'
import { cn } from '@/lib/utils'

interface CloudSyncDisclosureProps {
  className?: string
}

export function CloudSyncDisclosure({ className }: CloudSyncDisclosureProps) {
  const [termsLink, privacyLink, cloudSyncLink] = cloudSyncSignInLinks

  return (
    <p
      className={cn(
        'text-center text-muted-foreground text-xs leading-relaxed',
        className,
      )}
    >
      By signing in, you agree to the <DisclosureLink link={termsLink} /> and
      acknowledge the <DisclosureLink link={privacyLink} />.{' '}
      <DisclosureLink link={cloudSyncLink} />.
    </p>
  )
}

function DisclosureLink({
  link,
}: {
  link: (typeof cloudSyncSignInLinks)[number]
}) {
  return (
    <a
      href={link.url}
      target="_blank"
      rel="noopener noreferrer"
      className="font-medium underline underline-offset-2 hover:text-foreground"
    >
      {link.label}
    </a>
  )
}
