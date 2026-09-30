import UserNotifications

public class NotificationHandler: NSObject, NotificationHandlerProtocol {

  weak var plugin: NotificationPlugin?

  private var notificationsMap = [String: Notification]()
  private var hasClickedListener = false
  private var pendingNotificationClick: NotificationClickedData? = nil

  internal func saveNotification(_ key: String, _ notification: Notification) {
    notificationsMap.updateValue(notification, forKey: key)
  }

  func setClickListenerActive(_ active: Bool) {
    hasClickedListener = active

    if active, let pending = pendingNotificationClick {
      pendingNotificationClick = nil
      try? self.plugin?.trigger("notificationClicked", data: pending)
    }
  }

  public func requestPermissions() async throws -> Bool {
    let center = UNUserNotificationCenter.current()
    return try await center.requestAuthorization(options: [.badge, .alert, .sound])
  }

  public func checkPermissions() async -> UNNotificationSettings {
    let center = UNUserNotificationCenter.current()
    return await center.notificationSettings()
  }

  public func willPresent(notification: UNNotification) -> UNNotificationPresentationOptions {
    // Trigger notification event for both local and push notifications
    if var notificationData = toActiveNotification(notification.request) {
      notificationData.source = "local"
      try? self.plugin?.trigger("notification", data: notificationData)
    } else {
      var notificationData = toReceivedNotification(notification.request)
      notificationData.source = "push"
      try? self.plugin?.trigger("notification", data: notificationData)
    }

    return presentationOptions(for: notification.request)
  }

  func presentationOptions(for request: UNNotificationRequest) -> UNNotificationPresentationOptions {
    if request.trigger is UNPushNotificationTrigger {
      return []
    }
    if notificationsMap[request.identifier]?.silent == true {
      return [.badge, .alert]
    }
    return [.badge, .sound, .alert]
  }

  /// Convert notification request to ReceivedNotification (for push notifications not in map)
  private func toReceivedNotification(_ request: UNNotificationRequest) -> ReceivedNotificationData {
    let content = request.content
    var extra: [String: String]? = nil

    if !content.userInfo.isEmpty {
      extra = [:]
      for (key, value) in content.userInfo {
        if let keyStr = key as? String, let valStr = value as? String {
          extra?[keyStr] = valStr
        }
      }
      if extra?.isEmpty == true {
        extra = nil
      }
    }

    return ReceivedNotificationData(
      id: Int(request.identifier) ?? -1,
      title: content.title,
      body: content.body,
      extra: extra
    )
  }

  public func didReceive(response: UNNotificationResponse) {
    let originalNotificationRequest = response.notification.request
    let actionId = response.actionIdentifier

    var actionIdValue: String
    // We turn the two default actions (open/dismiss) into generic strings
    if actionId == UNNotificationDefaultActionIdentifier {
      actionIdValue = "tap"
    } else if actionId == UNNotificationDismissActionIdentifier {
      actionIdValue = "dismiss"
    } else {
      actionIdValue = actionId
    }

    var inputValue: String? = nil
    // If the type of action was for an input type, get the value
    if let inputType = response as? UNTextInputNotificationResponse {
      inputValue = inputType.userText
    }

    // Only trigger actionPerformed for local notifications (those in our map)
    if let activeNotification = toActiveNotification(originalNotificationRequest) {
      try? self.plugin?.trigger(
        "actionPerformed",
        data: ReceivedNotification(
          actionId: actionIdValue,
          inputValue: inputValue,
          notification: activeNotification
        ))
    }

    guard let clickedData = toNotificationClick(originalNotificationRequest, actionIdentifier: actionId) else {
      return
    }

    if hasClickedListener {
      try? self.plugin?.trigger("notificationClicked", data: clickedData)
    } else {
      pendingNotificationClick = clickedData
    }
  }

  func toNotificationClick(_ request: UNNotificationRequest, actionIdentifier: String) -> NotificationClickedData? {
    guard actionIdentifier == UNNotificationDefaultActionIdentifier else {
      return nil
    }

    let id = Int(request.identifier) ?? -1
    let userInfo = request.content.userInfo
    var dataDict: [String: String]? = nil
    if !userInfo.isEmpty {
      dataDict = [:]
      for (key, value) in userInfo {
        if let keyStr = key as? String, let valStr = value as? String {
          dataDict?[keyStr] = valStr
        }
      }
      if dataDict?.isEmpty == true {
        dataDict = nil
      }
    }

    return NotificationClickedData(id: id, data: dataDict)
  }

  func toActiveNotification(_ request: UNNotificationRequest) -> ActiveNotification? {
    guard let notificationRequest = notificationsMap[request.identifier] else {
      return nil
    }
    return ActiveNotification(
      id: Int(request.identifier) ?? -1,
      title: request.content.title,
      body: request.content.body,
      sound: notificationRequest.sound ?? "",
      actionTypeId: request.content.categoryIdentifier,
      attachments: notificationRequest.attachments,
      extra: notificationRequest.extra
    )
  }

  func toPendingNotification(_ request: UNNotificationRequest) -> PendingNotification? {
    guard let notification = notificationsMap[request.identifier] else {
      return nil
    }
    return PendingNotification(
      id: Int(request.identifier) ?? -1,
      title: request.content.title,
      body: request.content.body,
      schedule: notification.schedule!
    )
  }
}

struct PendingNotification: Encodable {
  let id: Int
  let title: String
  let body: String
  let schedule: NotificationSchedule
}

struct ActiveNotification: Encodable {
  let id: Int
  let title: String
  let body: String
  let sound: String
  let actionTypeId: String
  let attachments: [NotificationAttachment]?
  var extra: [String: String]? = nil
  var source: String = "local"
}

struct ReceivedNotification: Encodable {
  let actionId: String
  let inputValue: String?
  let notification: ActiveNotification
}

struct NotificationClickedData: Encodable {
  let id: Int
  let data: [String: String]?
}

struct ReceivedNotificationData: Encodable {
  let id: Int
  let title: String
  let body: String
  let extra: [String: String]?
  var source: String = "push"
}
