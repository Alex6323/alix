enum PickerDepth { recognize, recall, reconstruct }

class PickerDeadline {
  const PickerDeadline({
    required this.date,
    required this.daysLeft,
    required this.ready,
    required this.total,
  });

  final String date;
  final int daysLeft;
  final int ready;
  final int total;
}

class PickerProfile {
  const PickerProfile({required this.libMs, required this.counters});

  final int libMs;
  final List<(String, int)> counters;
}

class PickerListing {
  const PickerListing({required this.entries, this.deadline, this.profile});

  final List<PickerEntry> entries;
  final PickerDeadline? deadline;
  final PickerProfile? profile;
}

class PickerEntry {
  const PickerEntry({
    required this.title,
    required this.path,
    required this.isWorkspace,
    required this.due,
    required this.canRecognize,
    required this.isTrace,
    required this.lastDepth,
    required this.mastered,
    required this.examDue,
    required this.hasExam,
    required this.locked,
    required this.progressError,
    this.icon,
    required this.indent,
    required this.tree,
    this.deadline,
  });

  final String title;
  final String path;
  final bool isWorkspace;
  final bool due;
  final bool canRecognize;
  final bool isTrace;
  final PickerDepth lastDepth;
  final bool mastered;
  final bool examDue;
  final bool hasExam;
  final bool locked;
  final bool progressError;
  final String? icon;
  final int indent;
  final String tree;
  final PickerDeadline? deadline;
}
