import 'package:flutter_test/flutter_test.dart';

void main() {
  test('missed hit tests are fatal', () {
    expect(WidgetController.hitTestWarningShouldBeFatal, isTrue);
  });
}
