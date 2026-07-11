import { Navigate, Route, Routes } from 'react-router-dom'
import { AppLayout } from './components/AppLayout'
import { HomePage } from './pages/HomePage'
import { ModelsPage } from './pages/ModelsPage'
import { QuestionsPage } from './pages/QuestionsPage'
import { SettingsPage } from './pages/SettingsPage'
import './App.css'

function App() {
  return (
    <Routes>
      <Route element={<AppLayout />}>
        <Route path="/" element={<HomePage />} />
        <Route path="/questions" element={<QuestionsPage />} />
        <Route path="/models" element={<ModelsPage />} />
        <Route path="/settings" element={<SettingsPage />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Route>
    </Routes>
  )
}

export default App
