-- Original synthetic menu choreography for menu.sol. The screen table keeps
-- stable UI keys separate from the authored view names.
return {
  screens = {
    main = 'opening',
    single_player = 'fleet',
    setup = 'planning',
    mods = 'dock',
    settings = 'systems',
    multiplayer = 'rendezvous',
    missions = 'fleet',
  },
  campaigns = { federation = 'campaign' },

  views = {
    { name='opening', camera={'menu_camera_main','menu_camera_main_arc','menu_camera_main_return'}, target={'menu_target_main','menu_target_main_arc','menu_target_main_return'}, seconds=24, loop=true, easing='smooth', fov=39 },
    { name='fleet', camera={'menu_camera_fleet','menu_camera_fleet_arc','menu_camera_fleet_return'}, target={'menu_target_fleet','menu_target_fleet_arc','menu_target_fleet_return'}, seconds=18, loop=true, easing='smooth', fov=36 },
    { name='planning', camera={'menu_camera_planning','menu_camera_planning_arc','menu_camera_planning_return'}, target={'menu_target_planning','menu_target_planning_arc','menu_target_planning_return'}, seconds=21, loop=true, easing='smooth', fov=42 },
    { name='dock', camera={'menu_camera_dock','menu_camera_dock_arc','menu_camera_dock_return'}, target={'menu_target_dock','menu_target_dock_arc','menu_target_dock_return'}, seconds=20, loop=true, easing='smooth', fov=34 },
    { name='systems', camera={'menu_camera_settings','menu_camera_settings_arc','menu_camera_settings_return'}, target={'menu_target_settings','menu_target_settings_arc','menu_target_settings_return'}, seconds=19, loop=true, easing='smooth', fov=40 },
    { name='rendezvous', camera={'menu_camera_multiplayer','menu_camera_multiplayer_arc','menu_camera_multiplayer_return'}, target={'menu_target_multiplayer','menu_target_multiplayer_arc','menu_target_multiplayer_return'}, seconds=23, loop=true, easing='smooth', fov=38 },
    { name='campaign', camera={'menu_camera_campaign','menu_camera_campaign_arc','menu_camera_campaign_return'}, target={'menu_target_campaign','menu_target_campaign_arc','menu_target_campaign_return'}, seconds=27, loop=true, easing='smooth', fov=35 },
  },

  ships = {
    { name='escort', path={'escort','menu_point_escort_wide','menu_point_escort_close'}, seconds=32, loop=true, easing='smooth' },
    { name='flagship', path={'flagship','menu_point_flagship_wide','menu_point_flagship_close'}, seconds=40, loop=true, easing='smooth' },
  },

  stars=1800,
  blend=1.4,
}
