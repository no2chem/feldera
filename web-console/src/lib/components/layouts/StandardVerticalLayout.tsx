import VerticalNavItems from '$lib/functions/navigation/vertical'
import { ReactNode } from 'react'
import { useSettings } from 'src/@core/hooks/useSettings'
import VerticalLayout from 'src/@core/layouts/VerticalLayout'

import { Theme } from '@mui/material/styles'
import useMediaQuery from '@mui/material/useMediaQuery'

import VerticalAppBarContent from './vertical/AppBarContent'

interface Props {
  children: ReactNode
}

const StandardVerticalLayout = ({ children }: Props) => {
  const { settings, saveSettings } = useSettings()

  /**
   *  The below variable will hide the current layout menu at given screen size.
   *  The menu will be accessible from the Hamburger icon only (Vertical Overlay Menu).
   *  You can change the screen size from which you want to hide the current layout menu.
   *  Please refer useMediaQuery() hook: https://mui.com/components/use-media-query/,
   *  to know more about what values can be passed to this hook.
   */
  const hidden = useMediaQuery((theme: Theme) => theme.breakpoints.down('lg'))

  return (
    <VerticalLayout
      hidden={hidden}
      settings={settings}
      saveSettings={saveSettings}
      verticalNavItems={VerticalNavItems()} // Navigation Items
      verticalAppBarContent={(
        props // AppBar Content
      ) => (
        <VerticalAppBarContent
          hidden={hidden}
          settings={settings}
          saveSettings={saveSettings}
          toggleNavVisibility={props.toggleNavVisibility}
        />
      )}
    >
      {children}
    </VerticalLayout>
  )
}

export default StandardVerticalLayout
