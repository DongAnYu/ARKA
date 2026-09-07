import { Outlet } from 'react-router-dom'
import { AppSidebar } from './AppSidebar'
import { AppTitleBar } from './AppTitleBar'

export function AppLayout() {
  return (
    <div className="app-shell">
      <AppTitleBar />
      <div className="app-workspace">
        <AppSidebar />
        <main className="app-content">
          <Outlet />
        </main>
        <AppSidebar variant="mobile" />
      </div>
    </div>
  )
}
