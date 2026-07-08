import type { FC } from 'react'
import { getFavicons } from '@/lib/getFavicons'

interface TabPreviewCardProps {
  tab: chrome.tabs.Tab
  isSelected: boolean
  thumbnail?: string
  cardWidth: number
  cardHeight: number
}

function getHost(url?: string): string {
  try {
    return new URL(url ?? '').hostname
  } catch {
    return ''
  }
}

function getDomain(url?: string): string {
  const host = getHost(url)
  return host.startsWith('www.') ? host.slice(4) : host
}

export const TabPreviewCard: FC<TabPreviewCardProps> = ({
  tab,
  isSelected,
  thumbnail,
  cardWidth,
  cardHeight,
}) => {
  const thumbnailAreaHeight = cardHeight - 48

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        width: `${cardWidth}px`,
        height: `${cardHeight}px`,
        borderRadius: '12px',
        border: isSelected
          ? '2px solid rgba(255, 160, 50, 0.9)'
          : '1px solid rgba(255, 255, 255, 0.1)',
        padding: '8px',
        cursor: 'pointer',
        background: isSelected
          ? 'rgba(255, 255, 255, 0.12)'
          : 'rgba(255, 255, 255, 0.04)',
        backdropFilter: 'blur(20px)',
        WebkitBackdropFilter: 'blur(20px)',
        opacity: isSelected ? 1 : 0.75,
        boxShadow: isSelected
          ? '0 0 28px rgba(255, 160, 50, 0.4), 0 8px 32px rgba(0, 0, 0, 0.4)'
          : '0 2px 8px rgba(0, 0, 0, 0.2)',
        transform: isSelected ? 'scale(1.08)' : 'scale(1)',
        transition: 'all 0.15s ease-out',
        boxSizing: 'border-box',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          width: '100%',
          height: `${thumbnailAreaHeight}px`,
          overflow: 'hidden',
          borderRadius: '8px',
          background: 'rgba(0, 0, 0, 0.2)',
          flexShrink: 0,
        }}
      >
        {thumbnail ? (
          <img
            src={thumbnail}
            alt=""
            style={{
              width: '100%',
              height: '100%',
              objectFit: 'cover',
            }}
          />
        ) : (
          <img
            src={getFavicons(getHost(tab.url))}
            alt=""
            style={{
              width: '28px',
              height: '28px',
            }}
          />
        )}
      </div>

      <div
        style={{
          display: 'flex',
          alignItems: 'flex-start',
          gap: '6px',
          width: '100%',
          marginTop: '6px',
          minHeight: '0',
          flex: 1,
        }}
      >
        <img
          src={tab.favIconUrl ?? getFavicons(getHost(tab.url))}
          alt=""
          style={{
            width: '12px',
            height: '12px',
            borderRadius: '2px',
            flexShrink: 0,
            marginTop: '2px',
          }}
        />
        <span
          title={tab.title ?? getDomain(tab.url)}
          style={{
            fontSize: '10px',
            fontWeight: 500,
            lineHeight: 1.3,
            color: isSelected ? '#fff' : 'rgba(255, 255, 255, 0.8)',
            display: '-webkit-box',
            WebkitLineClamp: 2,
            WebkitBoxOrient: 'vertical',
            overflow: 'hidden',
            wordBreak: 'break-word',
          }}
        >
          {tab.title ?? getDomain(tab.url)}
        </span>
      </div>
    </div>
  )
}
