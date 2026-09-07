import { Navigate, Route, Routes } from 'react-router-dom'
import { AppLayout } from './components/AppLayout'
import { GenerationProvider } from './generation/GenerationContext'
import { HomePage } from './pages/HomePage'
import { ModelsPage } from './pages/ModelsPage'
import { QuestionsPage } from './pages/QuestionsPage'
import { SessionPage } from './pages/SessionPage'
import { SettingsPage } from './pages/SettingsPage'
import './App.css'

function App() {
  return (
    <GenerationProvider>
      <Routes>
        <Route element={<AppLayout />}>
          <Route path="/" element={<HomePage />} />
          <Route path="/questions" element={<QuestionsPage />} />
          <Route path="/session" element={<SessionPage />} />
          <Route path="/models" element={<ModelsPage />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Route>
      </Routes>
    </GenerationProvider>
  )
}

export default App
