import { FolderOpen, HelpCircle, Home, Plug, Settings } from 'lucide-react'
import { NavLink } from 'react-router-dom'
import arkaLogo from '../assets/arka-logo.svg'

const navItems = [
  { to: '/', label: 'Home', icon: Home, end: true },
  { to: '/session', label: 'Recall', icon: HelpCircle },
  { to: '/questions', label: 'Library', icon: FolderOpen },
  { to: '/models', label: 'Models', icon: Plug },
  { to: '/settings', label: 'Settings', icon: Settings },
]

type AppSidebarProps = {
  variant?: 'sidebar' | 'mobile'
}

export function AppSidebar({ variant = 'sidebar' }: AppSidebarProps) {
  if (variant === 'mobile') {
    return (
      <nav className="mobile-nav" aria-label="App navigation">
        {navItems.map(({ to, label, icon: Icon, end }) => (
          <NavLink
            key={to}
            to={to}
            end={end}
            className={({ isActive }) =>
              `mobile-nav-item${isActive ? ' is-active' : ''}`
            }
            aria-label={label}
          >
            <Icon className="size-4" aria-hidden="true" />
            <span>{label}</span>
          </NavLink>
        ))}
      </nav>
    )
  }

  return (
    <aside className="app-sidebar" aria-label="App navigation">
      <div className="sidebar-brand">
        <img src={arkaLogo} alt="Recall logo" className="sidebar-logo" />
        <span className="sidebar-title">Recall</span>
      </div>

      <nav className="sidebar-nav">
        {navItems.map(({ to, label, icon: Icon, end }) => (
          <NavLink
            key={to}
            to={to}
            end={end}
            className={({ isActive }) =>
              `sidebar-nav-item${isActive ? ' is-active' : ''}`
            }
          >
            <Icon className="size-4" aria-hidden="true" />
            {label}
          </NavLink>
        ))}
      </nav>
    </aside>
  )
}
